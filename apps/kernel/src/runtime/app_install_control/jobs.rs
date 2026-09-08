//! Slow work retains only the existing stores/services and bounded admission.
use super::*;
use crate::runtime::{app_lifecycle::LifecycleError, app_package_preparation::PreparationError};
pub(super) type Result = std::result::Result<Outcome, Error>;
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
        _ => Error::Failed("app_install_package_rejected"),
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
                let prepared = shared
                    .preparation
                    .prepare(
                        key.0.clone(),
                        input.upload_handle.clone(),
                        permit.take().unwrap(),
                    )
                    .await
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
