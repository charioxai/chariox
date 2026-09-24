use crate::local::*;
use crate::runtime::app_package_upload_control::{
    UploadCommand, UploadControlError, MAX_ENCODED_UPLOAD_CHUNK_BYTES,
};
use chariox_app_runtime::package_upload::{UploadPhase, UploadStatus};

pub(super) fn command(
    request: &LocalDaemonRequest,
) -> Result<Option<UploadCommand>, AppRequestErrorCode> {
    let upload = match request {
        LocalDaemonRequest::BeginAppPackageUpload(request) => {
            if request.request_id.len() > 128 || request.sha256.len() != 71 {
                return Err(AppRequestErrorCode::InvalidRequest);
            }
            UploadCommand::Begin {
                request_id: request.request_id.clone(),
                expected_size: request.expected_size,
                sha256: request.sha256.clone(),
            }
        }
        LocalDaemonRequest::PutAppPackageUploadChunk(request) => {
            // Bound before copying bytes into the admitted blocking task. The
            // command/audit projection and transport cache never copy the body.
            if request.data_base64.len() > MAX_ENCODED_UPLOAD_CHUNK_BYTES {
                return Err(AppRequestErrorCode::LimitExceeded);
            }
            if request.handle.len() != 71 {
                return Err(AppRequestErrorCode::NotFound);
            }
            if request.chunk_sha256.len() != 71 {
                return Err(AppRequestErrorCode::InvalidRequest);
            }
            UploadCommand::Chunk {
                handle: request.handle.clone(),
                offset: request.offset,
                data_base64: request.data_base64.clone(),
                chunk_sha256: request.chunk_sha256.clone(),
            }
        }
        LocalDaemonRequest::GetAppPackageUpload(request) => {
            if request.handle.len() != 71 {
                return Err(AppRequestErrorCode::NotFound);
            }
            UploadCommand::Status {
                handle: request.handle.clone(),
            }
        }
        LocalDaemonRequest::AbortAppPackageUpload(request) => {
            if request.handle.len() != 71 {
                return Err(AppRequestErrorCode::NotFound);
            }
            UploadCommand::Abort {
                handle: request.handle.clone(),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(upload))
}

pub(super) fn response(status: UploadStatus) -> LocalDaemonResponse {
    LocalDaemonResponse::AppPackageUploadStatus {
        upload: AppPackageUploadSummary {
            handle: status.handle,
            phase: match status.phase {
                UploadPhase::Receiving => AppPackageUploadPhase::Receiving,
                UploadPhase::Finalized => AppPackageUploadPhase::Finalized,
                UploadPhase::Aborted => AppPackageUploadPhase::Aborted,
            },
            expected_size: status.expected_size,
            accepted_bytes: status.accepted_bytes,
            sha256: status.sha256,
            expires_at_ms: status.expires_at_ms,
        },
    }
}

pub(super) fn error_code(error: UploadControlError) -> AppRequestErrorCode {
    match error {
        UploadControlError::InvalidRequest => AppRequestErrorCode::InvalidRequest,
        UploadControlError::NotFound => AppRequestErrorCode::NotFound,
        UploadControlError::Busy => AppRequestErrorCode::Busy,
        UploadControlError::LimitExceeded => AppRequestErrorCode::LimitExceeded,
        UploadControlError::Conflict => AppRequestErrorCode::Conflict,
        UploadControlError::DigestMismatch => AppRequestErrorCode::DigestMismatch,
        UploadControlError::StorageUnavailable => AppRequestErrorCode::StorageUnavailable,
    }
}
