//! Verified upload-to-release preparation, below terminal adapters. This owns no
//! second database, never grants capabilities, and never starts an App worker.

use std::fs::File;
use std::io::Read;
use std::sync::Arc;

use chariox_app_package::{inspect_untrusted, verify, ErrorCode, Limits, VerificationPolicy};
use chariox_app_runtime::{
    installation::{VerifiedInstallCandidate, VerifiedStageError},
    package_upload::{UploadError, MAX_UPLOAD_ARCHIVE_BYTES},
    publisher_trust::PublisherTrustError,
    release_store::{ReleaseStore, ReleaseStoreError, StageBudget, StagedRelease},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::app_package_upload_control::AppPackageUploadControl;
use crate::durable_state::{app_publishers::AppPublisherError, DurableKernelStateStore};

const MAX_STAGE_BYTES: u64 = 512 * 1024 * 1024;
const HOST_RESERVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Safe classifications, not raw I/O messages or paths. PublicationInterrupted
/// means retrying the same upload is safe: publication may already be visible,
/// and ReleaseStore revalidates/seals/syncs it before acknowledging reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparationError {
    InvalidRequest,
    UploadNotFound,
    UploadIncomplete,
    UploadConflict,
    UploadDigestMismatch,
    Busy,
    LimitExceeded,
    PublisherNotEnrolled,
    PublisherRevoked,
    PackageRejected(ErrorCode),
    InsufficientStorage,
    UnsafeRelease,
    PublicationInterrupted,
    StorageUnavailable,
}

type Result<T> = std::result::Result<T, PreparationError>;

/// Created only after enrolled-key verification and durable publication. Keep
/// this owner, immutable candidate and anchored directory together through the
/// next installer step. It is neither current authorization nor runtime health:
/// the candidate's enrollment revision is rechecked by the durable writer.
/// No display path or caller-supplied verification flag is exposed.
pub(crate) struct PreparedAppPackage {
    owner: String,
    candidate: VerifiedInstallCandidate,
    release: StagedRelease,
}

impl PreparedAppPackage {
    pub(crate) fn candidate(&self, trusted_owner: &str) -> Result<&VerifiedInstallCandidate> {
        self.require_owner(trusted_owner)?;
        Ok(&self.candidate)
    }

    pub(crate) fn directory(&self, trusted_owner: &str) -> Result<&File> {
        self.require_owner(trusted_owner)?;
        Ok(&self.release.directory)
    }

    pub(crate) fn reused(&self) -> bool {
        self.release.reused
    }

    fn require_owner(&self, owner: &str) -> Result<()> {
        if owner != self.owner {
            return Err(PreparationError::UploadNotFound);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct AppPackagePreparation {
    store: DurableKernelStateStore,
    uploads: AppPackageUploadControl,
    // Created once by the kernel service, shared by every service clone. A
    // single verification/publication bounds archive allocations and prevents
    // this service's own concurrent staging from overspending its reservation.
    preparation: Arc<Semaphore>,
}

impl AppPackagePreparation {
    /// Reuse the AppControl upload service, not a second open of its live store.
    /// Construct once per kernel and clone. Constructor performs no I/O.
    pub(crate) fn new(store: DurableKernelStateStore, uploads: AppPackageUploadControl) -> Self {
        Self {
            store,
            uploads,
            preparation: Arc::new(Semaphore::new(1)),
        }
    }

    /// Both admission permits are retained in the blocking task until all file
    /// and verification work finishes, even if its awaiting client disconnects.
    pub(crate) async fn prepare(
        &self,
        trusted_owner: String,
        upload_handle: String,
        admission: OwnedSemaphorePermit,
    ) -> Result<PreparedAppPackage> {
        let service = self.clone();
        admitted_blocking(Arc::clone(&self.preparation), admission, move || {
            service.prepare_at(
                &trusted_owner,
                &upload_handle,
                crate::session::unix_epoch_ms(),
                |_| Ok(()),
            )
        })
        .await
    }

    fn prepare_at(
        &self,
        owner: &str,
        handle: &str,
        now_ms: u64,
        mut checkpoint: impl FnMut(PreparationCheckpoint) -> Result<()>,
    ) -> Result<PreparedAppPackage> {
        // Finalize authenticates the owner/handle, rejects partial or expired
        // uploads, rehashes the exact file and retains its existing upload lease.
        let upload = self
            .uploads
            .finalize_for_preparation(owner, handle, now_ms)
            .map_err(upload_error)?;
        let expected = upload.status().expected_size;
        if expected == 0 || expected > MAX_UPLOAD_ARCHIVE_BYTES {
            return Err(PreparationError::LimitExceeded);
        }
        let capacity = usize::try_from(expected).map_err(|_| PreparationError::LimitExceeded)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| PreparationError::LimitExceeded)?;
        // Keep the anchored lease alive through verification and publication.
        // The extra byte detects inconsistent size without unbounded reading.
        upload
            .file()
            .take(expected + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PreparationError::StorageUnavailable)?;
        if bytes.len() != capacity {
            return Err(PreparationError::UploadDigestMismatch);
        }
        let claimed = inspect_untrusted(&bytes, &Limits::default())
            .map_err(|e| PreparationError::PackageRejected(e.code))?;
        let trust = self
            .store
            .trusted_app_publisher(
                owner,
                &claimed.manifest.publisher.id,
                &claimed.manifest.publisher.key_id,
            )
            .map_err(publisher_error)?;
        let policy = VerificationPolicy::new(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            vec![trust.publisher().clone()],
        );
        let verified =
            verify(&bytes, &policy).map_err(|e| PreparationError::PackageRejected(e.code))?;
        if verified.package_digest() != upload.status().sha256 {
            return Err(PreparationError::UploadDigestMismatch);
        }
        let candidate = VerifiedInstallCandidate::from_verified(&verified, &trust).map_err(
            |error| match error {
                VerifiedStageError::SignerMismatch => {
                    PreparationError::PackageRejected(ErrorCode::InvalidSignature)
                }
                _ => PreparationError::StorageUnavailable,
            },
        )?;
        checkpoint(PreparationCheckpoint::Verified)?;
        let releases = ReleaseStore::open_or_create(self.store.path())
            .map_err(|_| PreparationError::StorageUnavailable)?;
        releases.collect_abandoned().map_err(release_error)?;
        let reservation = releases
            .required_reservation(&verified, &bytes)
            .map_err(release_error)?;
        if reservation > MAX_STAGE_BYTES {
            return Err(PreparationError::LimitExceeded);
        }
        let release = releases
            .stage(
                &verified,
                &bytes,
                StageBudget {
                    max_stage_bytes: MAX_STAGE_BYTES,
                    reserved_bytes: reservation,
                    host_reserve_bytes: HOST_RESERVE_BYTES,
                },
            )
            .map_err(release_error)?;
        checkpoint(PreparationCheckpoint::Published)?;
        Ok(PreparedAppPackage {
            owner: owner.into(),
            candidate,
            release,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreparationCheckpoint {
    Verified,
    Published,
}

async fn admitted_blocking<T: Send + 'static>(
    preparation: Arc<Semaphore>,
    admission: OwnedSemaphorePermit,
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let reservation = preparation
        .try_acquire_owned()
        .map_err(|_| PreparationError::Busy)?;
    tokio::task::spawn_blocking(move || {
        let _admission = admission;
        let _reservation = reservation;
        operation()
    })
    .await
    .map_err(|_| PreparationError::StorageUnavailable)?
}

fn upload_error(error: UploadError) -> PreparationError {
    match error {
        UploadError::Invalid(_) => PreparationError::InvalidRequest,
        UploadError::NotFound => PreparationError::UploadNotFound,
        UploadError::Incomplete => PreparationError::UploadIncomplete,
        UploadError::Conflict => PreparationError::UploadConflict,
        UploadError::DigestMismatch => PreparationError::UploadDigestMismatch,
        UploadError::Busy => PreparationError::Busy,
        UploadError::Limit => PreparationError::LimitExceeded,
        _ => PreparationError::StorageUnavailable,
    }
}

fn publisher_error(error: AppPublisherError) -> PreparationError {
    match error {
        AppPublisherError::Trust(PublisherTrustError::NotFound) => {
            PreparationError::PublisherNotEnrolled
        }
        AppPublisherError::Trust(PublisherTrustError::Revoked) => {
            PreparationError::PublisherRevoked
        }
        _ => PreparationError::StorageUnavailable,
    }
}

fn release_error(error: ReleaseStoreError) -> PreparationError {
    match error {
        ReleaseStoreError::Busy => PreparationError::Busy,
        ReleaseStoreError::ReservationExceeded | ReleaseStoreError::EntryLimit => {
            PreparationError::LimitExceeded
        }
        ReleaseStoreError::HostReserve => PreparationError::InsufficientStorage,
        ReleaseStoreError::UnsafeEntry | ReleaseStoreError::InvalidExisting => {
            PreparationError::UnsafeRelease
        }
        ReleaseStoreError::ArchiveMismatch => {
            PreparationError::PackageRejected(ErrorCode::IntegrityMismatch)
        }
        ReleaseStoreError::InvalidRoot => PreparationError::StorageUnavailable,
        ReleaseStoreError::Io(_) => PreparationError::PublicationInterrupted,
    }
}

#[cfg(test)]
mod tests;
