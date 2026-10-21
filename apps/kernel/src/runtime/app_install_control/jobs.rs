//! Slow work retains only the existing stores/services and bounded admission.
use super::*;
use crate::runtime::{app_lifecycle::LifecycleError, app_package_preparation::PreparationError};
pub(super) type Result = std::result::Result<Outcome, Error>;
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve the existing Outcome typed actor payload layout"
)]
pub(super) enum Outcome {
    Review {
        operation: InstallOperation,
        review: InstallReviewDisposition,
    },
    Done,
    Stopped,
    Scan(Vec<Key>),
}
pub(super) enum Error {
    Busy,
    /// Every live worker slot is taken: an idle worker can make room.
    LiveLimit,
    Failed(&'static str),
    Storage,
    Unknown,
}
pub(super) enum Job {
    Work,
    Decide {
        challenge: Arc<InstallApprovalChallenge>,
        accepted: bool,
        deadline: Instant,
    },
    Start,
    Stop,
    Fail(&'static str),
    Scan(Option<Key>),
}
fn error(error: InstallOperationError) -> Error {
    match error {
        InstallOperationError::CommitUnknown => Error::Unknown,
        InstallOperationError::Storage => Error::Storage,
        InstallOperationError::Stopped => Error::Failed("app_install_cancelled_or_expired"),
        InstallOperationError::Stale => Error::Failed("app_install_authority_changed"),
        InstallOperationError::Limit => Error::Failed("app_install_limit"),
        InstallOperationError::NotFound => Error::Failed("app_install_not_found"),
        InstallOperationError::SchemaDowngrade => Error::Failed("app_update_schema_downgrade"),
        _ => Error::Failed("app_install_conflict"),
    }
}
fn preparation(error: PreparationError) -> Error {
    match error {
        PreparationError::Busy => Error::Busy,
        PreparationError::PublisherNotEnrolled => {
            Error::Failed("app_install_publisher_not_enrolled")
        }
        PreparationError::PublisherRevoked => Error::Failed("app_install_publisher_revoked"),
        PreparationError::UploadNotFound => Error::Failed("app_install_upload_missing_or_expired"),
        PreparationError::UploadIncomplete => Error::Failed("app_install_upload_incomplete"),
        PreparationError::InsufficientStorage => Error::Failed("app_install_insufficient_storage"),
        PreparationError::StorageUnavailable | PreparationError::PublicationInterrupted => {
            Error::Storage
        }
        PreparationError::PackageRejected(code) => Error::Failed(package_failure(code)),
        PreparationError::InvalidRequest => Error::Failed("app_install_invalid_request"),
        PreparationError::UploadConflict => Error::Failed("app_install_upload_aborted"),
        PreparationError::UploadDigestMismatch => {
            Error::Failed("app_install_upload_digest_mismatch")
        }
        PreparationError::LimitExceeded => Error::Failed("app_install_release_limit"),
        PreparationError::UnsafeRelease => Error::Failed("app_install_release_unsafe"),
        PreparationError::ArchiveMismatch => Error::Failed("app_install_release_archive_mismatch"),
    }
}

/// The package's stable error code reaches clients (V-PKG-03), so a protocol,
/// SDK or manifest mismatch reads differently from a bad signature.
fn package_failure(code: chariox_app_package::ErrorCode) -> &'static str {
    use chariox_app_package::ErrorCode;
    match code {
        ErrorCode::InvalidArguments => "app_install_package_invalid_arguments",
        ErrorCode::Io => "app_install_package_io",
        ErrorCode::InvalidDeveloperKey => "app_install_package_invalid_developer_key",
        ErrorCode::InvalidArchive => "app_install_package_invalid_archive",
        ErrorCode::ArchiveLimit => "app_install_package_archive_limit",
        ErrorCode::InvalidPath => "app_install_package_invalid_path",
        ErrorCode::DuplicatePath => "app_install_package_duplicate_path",
        ErrorCode::InvalidManifest => "app_install_package_invalid_manifest",
        ErrorCode::InvalidSchema => "app_install_package_invalid_schema",
        ErrorCode::IncompatibleProtocol => "app_install_package_incompatible_protocol",
        ErrorCode::IncompatibleSdk => "app_install_package_incompatible_sdk",
        ErrorCode::IncompatibleContract => "app_install_package_incompatible_contract",
        ErrorCode::IncompatibleResourcePolicy => "app_install_package_incompatible_resource_policy",
        ErrorCode::UntrustedPublisher => "app_install_package_untrusted_publisher",
        ErrorCode::InvalidSignature => "app_install_package_invalid_signature",
        ErrorCode::IntegrityMismatch => "app_install_package_integrity_mismatch",
        ErrorCode::MissingEntry => "app_install_package_missing_entry",
        ErrorCode::UnexpectedEntry => "app_install_package_unexpected_entry",
        ErrorCode::UnsupportedFeature => "app_install_package_unsupported_feature",
    }
}
fn budget(cancelled: Arc<AtomicBool>) -> AppOperationBudget {
    AppOperationBudget::from_supervisor(move || cancelled.load(Ordering::Acquire))
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> std::result::Result<T, InstallOperationError> + Send + 'static,
) -> std::result::Result<T, Error> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| Error::Unknown)?
        .map_err(error)
}
pub(super) async fn run(
    shared: Arc<Shared>,
    key: Key,
    cancelled: Arc<AtomicBool>,
    job: Job,
) -> Result {
    let permit = shared
        .admission
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Busy)?;
    match job {
        Job::Scan(after) => {
            let store = shared.store.clone();
            blocking(move || {
                let _permit = permit;
                store.pending_public_app_installs(
                    after.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
                )
            })
            .await
            .map(Outcome::Scan)
        }
        Job::Work => {
            let store = shared.store.clone();
            let owner = key.0.clone();
            let request = key.1.clone();
            let (mut operation, mut permit) = blocking(move || {
                let value = store.first_app_install_status(&owner, &request)?;
                Ok((value, Some(permit)))
            })
            .await?;
            if cancelled.load(Ordering::Acquire) {
                return Ok(Outcome::Done);
            }
            if operation.phase == InstallPhase::Preparing {
                let input = operation
                    .input
                    .as_ref()
                    .ok_or(Error::Failed("app_install_input_missing"))?;
                let upload_handle = input.upload_handle.clone();
                let deployment = input.deployment.is_some();
                let preparing = permit.take().unwrap();
                let prepared = if deployment {
                    shared
                        .preparation
                        .prepare_release(key.0.clone(), operation.package_digest.clone(), preparing)
                        .await
                } else {
                    shared
                        .preparation
                        .prepare(key.0.clone(), upload_handle.clone(), preparing)
                        .await
                }
                .map_err(preparation)?;
                let admitted = shared
                    .admission
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| Error::Busy)?;
                let store = shared.store.clone();
                let owner = key.0.clone();
                let request = key.1.clone();
                let limit = budget(cancelled.clone());
                operation = blocking(move || {
                    let _permit = admitted;
                    let candidate = prepared
                        .candidate(&owner)
                        .map_err(|_| InstallOperationError::Stale)?
                        .clone();
                    store.complete_app_install_preparation(&owner, &request, candidate, limit)
                })
                .await?;
                // The staged release is durable and past preparation: a restart
                // never prepares from this upload again.
                if !deployment {
                    let (preparation, owner) = (shared.preparation.clone(), key.0.clone());
                    let _ = tokio::task::spawn_blocking(move || {
                        preparation.release_upload(&owner, &upload_handle)
                    })
                    .await;
                }
                permit = Some(
                    shared
                        .admission
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| Error::Busy)?,
                );
            }
            if !matches!(
                operation.phase,
                InstallPhase::AwaitingApproval | InstallPhase::Starting
            ) {
                return Ok(Outcome::Done);
            }
            let store = shared.store.clone();
            let owner = key.0;
            let request = key.1;
            let limit = budget(cancelled);
            let interaction = format!("app_install_{:032x}", rand::random::<u128>());
            let review = blocking(move || {
                let _permit = permit;
                store.arm_app_install_review(&owner, &request, &interaction, limit)
            })
            .await?;
            Ok(Outcome::Review { operation, review })
        }
        Job::Decide {
            challenge,
            accepted,
            deadline,
        } => {
            let store = shared.store.clone();
            // Captured before registration, never recomputed on scheduling or
            // SQLite delays. The 30s writer limit only shortens that deadline.
            let limit = AppOperationBudget::from_supervisor(move || {
                cancelled.load(Ordering::Acquire) || Instant::now() >= deadline
            });
            let operation = blocking(move || {
                let _permit = permit;
                store.decide_app_install(challenge, accepted, limit)
            })
            .await?;
            Ok(if accepted {
                Outcome::Review {
                    operation,
                    review: InstallReviewDisposition::Approved,
                }
            } else {
                Outcome::Done
            })
        }
        Job::Start | Job::Stop => {
            let lifecycle = shared.lifecycle.clone();
            let store = shared.store.clone();
            let runtime = tokio::runtime::Handle::current();
            let stopping = matches!(job, Job::Stop);
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let operation = store
                    .first_app_install_status(&key.0, &key.1)
                    .map_err(|_| LifecycleError::Storage)?;
                if stopping {
                    // A cancellation observed during saturated admission still
                    // owns its durable cancellation; a later pass cannot merely
                    // stop an absent worker and leave Preparing recoverable.
                    if operation.phase == InstallPhase::Committed {
                        return Ok(());
                    }
                    let operation = store
                        .cancel_first_app_install(
                            &key.0,
                            &key.1,
                            AppOperationBudget::from_supervisor(|| false),
                        )
                        .map_err(|_| LifecycleError::Storage)?;
                    if operation.review.is_none() {
                        return Ok(());
                    }
                    lifecycle.stop_blocking(&key.0, &operation.token.installation_id)
                } else {
                    if cancelled.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    lifecycle
                        .start_first_blocking(&key.0, &key.1, runtime)
                        .map(|_| ())
                }
            })
            .await
            .map_err(|_| Error::Unknown)?;
            match result {
                Ok(()) => Ok(if stopping {
                    Outcome::Stopped
                } else {
                    Outcome::Done
                }),
                Err(LifecycleError::LiveLimit) => Err(Error::LiveLimit),
                Err(LifecycleError::Busy) => Err(Error::Busy),
                Err(LifecycleError::CommitUnknown) => Err(Error::Unknown),
                Err(LifecycleError::Storage) => Err(Error::Storage),
                Err(_) => Err(Error::Failed("app_install_start_rejected")),
            }
        }
        Job::Fail(code) => {
            let store = shared.store.clone();
            blocking(move || {
                let _permit = permit;
                store.fail_app_install(
                    &key.0,
                    &key.1,
                    code,
                    AppOperationBudget::from_supervisor(|| false),
                )
            })
            .await?;
            Ok(Outcome::Done)
        }
    }
}

#[cfg(test)]
mod package_failure_tests {
    use super::package_failure;
    use chariox_app_package::ErrorCode;

    #[test]
    fn each_package_error_code_reaches_clients_as_its_own_failure() {
        let codes = [
            ErrorCode::InvalidArguments,
            ErrorCode::Io,
            ErrorCode::InvalidDeveloperKey,
            ErrorCode::InvalidArchive,
            ErrorCode::ArchiveLimit,
            ErrorCode::InvalidPath,
            ErrorCode::DuplicatePath,
            ErrorCode::InvalidManifest,
            ErrorCode::InvalidSchema,
            ErrorCode::IncompatibleProtocol,
            ErrorCode::IncompatibleSdk,
            ErrorCode::IncompatibleContract,
            ErrorCode::IncompatibleResourcePolicy,
            ErrorCode::UntrustedPublisher,
            ErrorCode::InvalidSignature,
            ErrorCode::IntegrityMismatch,
            ErrorCode::MissingEntry,
            ErrorCode::UnexpectedEntry,
            ErrorCode::UnsupportedFeature,
        ];
        let failures = codes.map(package_failure);
        for (code, failure) in codes.iter().zip(failures) {
            // Same spelling as the package's serialized code, in the clients'
            // `app_(install|update)_[a-z_]+` failure format.
            let serialized = serde_json::to_value(code).unwrap();
            let expected = format!(
                "app_install_package_{}",
                serialized.as_str().unwrap().to_lowercase()
            );
            assert_eq!(failure, expected);
        }
        let distinct = failures.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(distinct.len(), codes.len());
    }
}
