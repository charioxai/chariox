use crate::{
    durable_state::app_state::AppStateError, runtime::app_operation_budget::AppOperationStopped,
};
use chariox_app_runtime::{app_outbox::OutboxError, managed_state::StateError, wire::RemoteError};

fn error(code: &str, message: &str, retryable: bool) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: message.into(),
        retryable: Some(retryable),
    }
}
pub(super) fn invalid() -> RemoteError {
    error("INVALID_ARGUMENT", "Invalid App storage request", false)
}
pub(super) fn limit() -> RemoteError {
    error(
        "LIMIT_EXCEEDED",
        "App storage request exceeds its limit",
        false,
    )
}
pub(super) fn busy() -> RemoteError {
    error("BUSY", "Kernel App operation capacity is full", true)
}
pub(super) fn unavailable() -> RemoteError {
    error("UNAVAILABLE", "App storage is unavailable", true)
}
pub(super) fn unknown_method() -> RemoteError {
    error("METHOD_NOT_FOUND", "Unknown App storage operation", false)
}
pub(super) fn stopped(error_value: AppOperationStopped) -> RemoteError {
    match error_value {
        AppOperationStopped::Cancelled => {
            error("CANCELLED", "App storage request was cancelled", false)
        }
        AppOperationStopped::Deadline => {
            error("DEADLINE_EXCEEDED", "App storage deadline exceeded", false)
        }
    }
}
pub(super) fn changes(error_value: StateError) -> RemoteError {
    match error_value {
        StateError::Invalid => invalid(),
        StateError::Limit => limit(),
        StateError::Conflict => error("CONFLICT", "App state version check failed", true),
        StateError::SchemaMismatch => error(
            "SCHEMA_MISMATCH",
            "App state schema version does not match",
            false,
        ),
        StateError::Installation(_) => error(
            "APP_UNAVAILABLE",
            "App state is not available for this worker",
            false,
        ),
        StateError::Database(_) | StateError::Corrupt => unavailable(),
    }
}
pub(super) fn state(error_value: AppStateError) -> RemoteError {
    match error_value {
        AppStateError::Stopped(reason) => stopped(reason),
        AppStateError::State(reason) => changes(reason),
        AppStateError::Outbox(reason) => outbox(reason),
        AppStateError::Catalog(_) => error(
            "APP_UNAVAILABLE",
            "App state is not available for this worker",
            false,
        ),
        AppStateError::Storage(_) => unavailable(),
    }
}

pub(super) fn outbox(reason: OutboxError) -> RemoteError {
    match reason {
        OutboxError::Invalid => invalid(),
        OutboxError::Limit => limit(),
        OutboxError::NotFound => error(
            "NOT_FOUND",
            "App event receipt or automation is unavailable",
            false,
        ),
        OutboxError::Conflict => {
            error("CONFLICT", "App event receipt or automation changed", false)
        }
        OutboxError::Inactive => error("AUTOMATION_INACTIVE", "App automation is inactive", false),
        OutboxError::Schema => error(
            "SCHEMA_MISMATCH",
            "App occurrence does not match its declared event",
            false,
        ),
        OutboxError::TooOld => error(
            "OCCURRENCE_TOO_OLD",
            "App occurrence is outside its admission window",
            false,
        ),
        OutboxError::Catalog(_) => error(
            "APP_UNAVAILABLE",
            "App storage is not available for this worker",
            false,
        ),
        OutboxError::Database(_) | OutboxError::Corrupt => unavailable(),
    }
}
