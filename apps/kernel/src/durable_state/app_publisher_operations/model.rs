use super::*;
use chariox_app_package::TrustedPublisher;
use chariox_app_runtime::publisher_trust::TrustDecisionReceipt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PublisherOperationError {
    #[error("app_publisher_operation_invalid")]
    Invalid,
    #[error("app_publisher_operation_not_found")]
    NotFound,
    #[error("app_publisher_operation_conflict")]
    Conflict,
    #[error("app_publisher_operation_limit")]
    Limit,
    #[error("app_publisher_operation_stopped")]
    Stopped,
    #[error("app_publisher_operation_storage")]
    Storage,
    #[error("app_publisher_operation_commit_unknown")]
    CommitUnknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublisherEnrollmentInput {
    pub(crate) session_id: String,
    pub(crate) publisher_id: String,
    pub(crate) key_id: String,
    pub(crate) public_key: [u8; 32],
    pub(crate) expected_revision: u64,
}
impl PublisherEnrollmentInput {
    pub(super) fn publisher(&self) -> Result<TrustedPublisher> {
        text(&self.session_id)?;
        for id in [&self.publisher_id, &self.key_id] {
            text(id)?;
            if !id.as_bytes()[0].is_ascii_alphanumeric()
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err(PublisherOperationError::Invalid);
            }
        }
        if self.expected_revision > i64::MAX as u64 {
            return Err(PublisherOperationError::Invalid);
        }
        let public_key = ed25519_dalek::VerifyingKey::from_bytes(&self.public_key)
            .map_err(|_| PublisherOperationError::Invalid)?;
        if public_key.is_weak() {
            return Err(PublisherOperationError::Invalid);
        }
        Ok(TrustedPublisher {
            publisher_id: self.publisher_id.clone(),
            key_id: self.key_id.clone(),
            public_key,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublisherOperationPhase {
    Pending,
    Approved,
    Denied,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublisherOperation {
    pub(crate) request_id: String,
    pub(crate) input: PublisherEnrollmentInput,
    pub(crate) phase: PublisherOperationPhase,
    pub(crate) interaction_id: Option<String>,
    /// Historical decision only; later revocation can have superseded it.
    pub(crate) receipt: Option<TrustDecisionReceipt>,
    pub(crate) failure: Option<String>,
}
pub(crate) struct PublisherApprovalChallenge {
    pub(super) owner: String,
    pub(super) request: String,
    pub(super) interaction: String,
    pub(super) input: PublisherEnrollmentInput,
    pub(super) deadline: Instant,
}
impl PublisherApprovalChallenge {
    pub(crate) fn interaction_id(&self) -> &str {
        &self.interaction
    }
    pub(crate) fn input(&self) -> &PublisherEnrollmentInput {
        &self.input
    }
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }
}
pub(crate) enum PublisherReview {
    Prompt(PublisherApprovalChallenge),
    Terminal(PublisherOperation),
}
pub(super) fn text(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(PublisherOperationError::Invalid)
    } else {
        Ok(())
    }
}
