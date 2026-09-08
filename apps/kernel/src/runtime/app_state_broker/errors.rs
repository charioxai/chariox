use crate::{
    durable_state::app_state::AppStateError, runtime::app_operation_budget::AppOperationStopped,
};
use chariox_app_runtime::{managed_state::StateError, wire::RemoteError};

fn error(code: &str, message: &str, retryable: bool) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: message.into(),
        retryable: Some(retryable),
    }
}
pub(super) fn invalid() -> RemoteError {
    error("INVALID_ARGUMENT", "Invalid App state request", false)
}
pub(super) fn limit() -> RemoteError {
    error(
        "LIMIT_EXCEEDED",
        "App state request exceeds its limit",
        false,
    )
}
pub(super) fn busy() -> RemoteError {
    error("BUSY", "Kernel App operation capacity is full", true)
}
pub(super) fn unavailable() -> RemoteError {
    error("UNAVAILABLE", "App state storage is unavailable", true)
}
pub(super) fn unknown_method() -> RemoteError {
    error("METHOD_NOT_FOUND", "Unknown App state operation", false)
}
pub(super) fn unsupported_occurrences() -> RemoteError {
    error(
        "UNSUPPORTED_OPERATION",
        "State transactions with event occurrences are not available",
        false,
    )
}
pub(super) fn stopped(error_value: AppOperationStopped) -> RemoteError {
    match error_value {
        AppOperationStopped::Cancelled => {
            error("CANCELLED", "App state request was cancelled", false)
        }
        AppOperationStopped::Deadline => {
            error("DEADLINE_EXCEEDED", "App state deadline exceeded", false)
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
        AppStateError::Catalog(_) => error(
            "APP_UNAVAILABLE",
            "App state is not available for this worker",
            false,
        ),
        AppStateError::Storage(_) => unavailable(),
    }
}
