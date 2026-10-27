//! Public review input passes through the same authenticated terminal route.
//! Only the private kernel interaction can turn that input into a trust grant.
use super::*;
use crate::{local::*, runtime::command::KernelCommand};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};

fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    LocalDaemonResponse::AppRequestFailed { code }
}
fn error(value: PublisherOperationError) -> LocalDaemonResponse {
    failed(match value {
        PublisherOperationError::Invalid => AppRequestErrorCode::InvalidRequest,
        PublisherOperationError::NotFound => AppRequestErrorCode::NotFound,
        PublisherOperationError::Conflict => AppRequestErrorCode::Conflict,
        PublisherOperationError::Limit => AppRequestErrorCode::LimitExceeded,
        PublisherOperationError::Busy => AppRequestErrorCode::Busy,
        PublisherOperationError::Storage
        | PublisherOperationError::CommitUnknown
        | PublisherOperationError::Stopped => AppRequestErrorCode::StorageUnavailable,
    })
}
fn input(value: &BeginAppPublisherEnrollmentRequest) -> Result<PublisherEnrollmentInput, ()> {
    if value.public_key_base64.len() != 44
        || value.expected_revision.len() > 19
        || value.request_id.len() > 128
        || value.publisher_id.len() > 128
        || value.key_id.len() > 128
    {
        return Err(());
    }
    let public_key: [u8; 32] = STANDARD
        .decode(&value.public_key_base64)
        .map_err(|_| ())?
        .try_into()
        .map_err(|_| ())?;
    let expected_revision: u64 = value.expected_revision.parse().map_err(|_| ())?;
    if STANDARD.encode(public_key) != value.public_key_base64
        || expected_revision.to_string() != value.expected_revision
        || expected_revision > i64::MAX as u64
    {
        return Err(());
    }
    Ok(PublisherEnrollmentInput {
        session_id: value.session_id.clone(),
        publisher_id: value.publisher_id.clone(),
        key_id: value.key_id.clone(),
        public_key,
        expected_revision,
    })
}
fn projection(value: PublisherOperation) -> LocalDaemonResponse {
    LocalDaemonResponse::AppPublisherEnrollmentStatus {
        operation: AppPublisherEnrollmentSummary {
            request_id: value.request_id,
            phase: match value.phase {
                PublisherOperationPhase::Pending => AppPublisherEnrollmentPhase::Pending,
                PublisherOperationPhase::Approved => AppPublisherEnrollmentPhase::Approved,
                PublisherOperationPhase::Denied => AppPublisherEnrollmentPhase::Denied,
                PublisherOperationPhase::Cancelled => AppPublisherEnrollmentPhase::Cancelled,
                PublisherOperationPhase::Failed => AppPublisherEnrollmentPhase::Failed,
            },
            key_fingerprint: format!("sha256:{:x}", Sha256::digest(value.input.public_key)),
            publisher_id: value.input.publisher_id,
            key_id: value.input.key_id,
            approved_revision: value
                .receipt
                .filter(|r| r.enrolled)
                .map(|r| r.revision.to_string()),
            interaction_id: value.interaction_id,
            failure: value.failure,
        },
    }
}
impl AppPublisherControl {
    pub(crate) async fn execute(
        &self,
        runtime: &KernelRuntimeState,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<LocalDaemonResponse> {
        if !matches!(
            request,
            LocalDaemonRequest::BeginAppPublisherEnrollment(_)
                | LocalDaemonRequest::GetAppPublisherEnrollment(_)
                | LocalDaemonRequest::CancelAppPublisherEnrollment(_)
        ) {
            return None;
        }
        let owner = match super::super::app_control::owner(command) {
            Ok(owner) => owner,
            Err(code) => return Some(failed(code)),
        };
        let result = match request {
            LocalDaemonRequest::BeginAppPublisherEnrollment(value) => {
                if value.session_id.len() > 128 {
                    return Some(failed(AppRequestErrorCode::InvalidRequest));
                }
                if !runtime.app_install_session_member(&value.session_id, &owner) {
                    return Some(failed(AppRequestErrorCode::Unauthorized));
                }
                let input = match input(value) {
                    Ok(input) => input,
                    Err(()) => return Some(failed(AppRequestErrorCode::InvalidRequest)),
                };
                self.begin(runtime, &owner, &value.request_id, input).await
            }
            LocalDaemonRequest::GetAppPublisherEnrollment(value) => {
                self.status(&owner, &value.request_id).await
            }
            LocalDaemonRequest::CancelAppPublisherEnrollment(value) => {
                self.cancel(&owner, &value.request_id).await
            }
            _ => unreachable!(),
        };
        Some(result.map(projection).unwrap_or_else(error))
    }
}
