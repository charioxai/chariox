use super::*;
pub(super) type Result = std::result::Result<Outcome, Error>;
pub(super) enum Outcome {
    Review(PublisherReview),
    Terminal,
    Scan(Vec<Key>),
}
pub(super) enum Error {
    Busy,
    Storage,
    Unknown,
    Stopped,
}
pub(super) enum Job {
    Arm,
    Decide {
        challenge: Arc<PublisherApprovalChallenge>,
        accepted: bool,
    },
    Cancel,
    Scan(Option<Key>),
}
pub(super) fn error(error: PublisherOperationError) -> Error {
    match error {
        PublisherOperationError::Limit => Error::Busy,
        PublisherOperationError::Storage => Error::Storage,
        PublisherOperationError::CommitUnknown => Error::Unknown,
        _ => Error::Stopped,
    }
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
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let budget = || {
            let cancelled = cancelled.clone();
            AppOperationBudget::from_supervisor(move || cancelled.load(Ordering::Acquire))
        };
        match job {
            Job::Arm => {
                let current = shared
                    .store
                    .publisher_enrollment_status(&key.0, &key.1, budget())
                    .map_err(error)?;
                if current.phase != PublisherOperationPhase::Pending {
                    return Ok(Outcome::Terminal);
                }
                let interaction = format!("app_publisher_{:032x}", rand::random::<u128>());
                shared
                    .store
                    .arm_publisher_enrollment(
                        &key.0,
                        &key.1,
                        &interaction,
                        Instant::now() + Duration::from_secs(300),
                        budget(),
                    )
                    .map(Outcome::Review)
                    .map_err(error)
            }
            Job::Decide {
                challenge,
                accepted,
            } => shared
                .store
                .decide_publisher_enrollment(challenge, accepted, budget())
                .map(|_| Outcome::Terminal)
                .map_err(error),
            Job::Cancel => {
                let result = shared.store.cancel_publisher_enrollment(
                    &key.0,
                    &key.1,
                    AppOperationBudget::from_supervisor(|| false),
                );
                match result {
                    Ok(_) => Ok(Outcome::Terminal),
                    Err(PublisherOperationError::NotFound) => Ok(Outcome::Terminal),
                    Err(PublisherOperationError::Conflict) => {
                        // Cancellation cannot revoke a decision already committed.
                        let current = shared
                            .store
                            .publisher_enrollment_status(
                                &key.0,
                                &key.1,
                                AppOperationBudget::from_supervisor(|| false),
                            )
                            .map_err(error)?;
                        if current.phase != PublisherOperationPhase::Pending {
                            Ok(Outcome::Terminal)
                        } else {
                            Err(Error::Storage)
                        }
                    }
                    Err(value) => Err(error(value)),
                }
            }
            Job::Scan(after) => shared
                .store
                .pending_publisher_enrollments(
                    after.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
                )
                .map(Outcome::Scan)
                .map_err(error),
        }
    })
    .await
    .map_err(|_| Error::Unknown)?
}
