//! The code and message an App tool call's caller sees when it fails, the same
//! for a view and for an agent: the App's own error passes through; a result
//! that breaks the tool's declared output and a missed call deadline have
//! their own codes; other kernel-side failures stay a generic `APP_ERROR`.
use crate::durable_state::app_tools::AppToolsError;
use crate::runtime::app_worker::AppWorkerError;
use chariox_app_runtime::app_catalog::CatalogError;

const INVALID_OUTPUT: (&str, &str) = (
    "INVALID_OUTPUT",
    "The App's answer does not match the tool's declared output",
);
const DEADLINE: (&str, &str) = ("DEADLINE_EXCEEDED", "The App did not answer in time");

pub(crate) fn tool_call_error(error: &AppToolsError) -> (String, String) {
    match error {
        AppToolsError::Catalog(CatalogError::Worker(remote)) => {
            (remote.code.clone(), remote.message.clone())
        }
        AppToolsError::Catalog(CatalogError::Output) => owned(INVALID_OUTPUT),
        AppToolsError::Catalog(CatalogError::Deadline) => owned(DEADLINE),
        AppToolsError::Worker(error) => worker_call_error(error),
        error => ("APP_ERROR".into(), error.to_string()),
    }
}

/// The kernel's shared App admission is momentarily full. The App never saw
/// the call, so trying again is safe; this is not a stopped App.
pub(crate) fn admission_busy() -> (String, String) {
    owned((
        "APP_BUSY",
        "Chariox is busy with other App requests; try again",
    ))
}

/// The App answered, but its answer could not be recorded in time. It may
/// have acted, so the caller checks before trying again.
pub(crate) fn reply_unrecorded() -> (String, String) {
    owned((
        "APP_OUTCOME_UNCERTAIN",
        "The App ran the call but its answer could not be recorded; check before trying again",
    ))
}

pub(crate) fn worker_call_error(error: &AppWorkerError) -> (String, String) {
    match error {
        AppWorkerError::Deadline => owned(DEADLINE),
        error => ("APP_ERROR".into(), error.to_string()),
    }
}

fn owned((code, message): (&str, &str)) -> (String, String) {
    (code.into(), message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caller_sees_typed_codes_for_bad_output_and_a_missed_deadline() {
        let code = |error: AppToolsError| tool_call_error(&error).0;
        assert_eq!(
            code(AppToolsError::Catalog(CatalogError::Output)),
            "INVALID_OUTPUT"
        );
        assert_eq!(
            code(AppToolsError::Catalog(CatalogError::Deadline)),
            "DEADLINE_EXCEEDED"
        );
        assert_eq!(
            code(AppToolsError::Worker(AppWorkerError::Deadline)),
            "DEADLINE_EXCEEDED"
        );
        assert_eq!(
            worker_call_error(&AppWorkerError::Deadline).0,
            "DEADLINE_EXCEEDED"
        );
        assert_eq!(
            code(AppToolsError::Catalog(CatalogError::Schema)),
            "APP_ERROR"
        );
        assert_eq!(worker_call_error(&AppWorkerError::Busy).0, "APP_ERROR");
    }

    #[test]
    fn full_admission_is_busy_before_a_call_and_uncertain_after_its_answer() {
        assert_eq!(admission_busy().0, "APP_BUSY");
        assert_eq!(reply_unrecorded().0, "APP_OUTCOME_UNCERTAIN");
        assert!(!admission_busy().1.contains("not running"));
    }
}
