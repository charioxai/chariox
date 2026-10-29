//! MP-08/MP-10/MP-11: keep kernel dispatch diagnostics out of agent transcripts.

use crate::error::DaemonError;

pub(crate) fn is_internal_dispatch_diagnostic(message: &str) -> bool {
    [
        "Provider prompt dispatch failed: ",
        "Prompt dispatch failed after acknowledgement: ",
        "Request not carried out: ",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
}

pub(crate) fn user_facing_dispatch_failure(error: &DaemonError) -> String {
    let message = match error {
        DaemonError::ProviderProtocol { operation, .. } => operation_failure_summary(operation),
        DaemonError::LocalTransport { message, .. } | DaemonError::ProviderRefusal { message } => {
            return crate::provider::sanitize_provider_diagnostic(message);
        }
        _ => "The provider could not complete the request.",
    };
    message.to_string()
}

/// The provider's own failure when it refused the dispatch. A kernel-side
/// error carries none and stays an internal diagnostic.
pub(crate) fn provider_dispatch_failure(
    adapter_key: &str,
    error: &DaemonError,
) -> Option<crate::provider::TurnOutcome> {
    match error {
        DaemonError::ProviderRefusal { message } => Some(
            crate::provider::TurnOutcome::classify(adapter_key, message),
        ),
        _ => None,
    }
}

fn operation_failure_summary(operation: &str) -> &'static str {
    match operation {
        "session_permissions" => "The provider could not confirm session permissions.",
        "provider_catalog" | "provider_configuration" => {
            "The provider's model list is unavailable."
        }
        "configuration_read" => "The provider's configuration is unavailable.",
        "agent_catalog" => "The provider's agent list is unavailable.",
        "mcp_status" | "mcp_registration" | "mcp_connect" | "opencode_mcp_ready" => {
            "The provider's Chariox tool connection is unavailable."
        }
        _ => "The provider could not confirm the request.",
    }
}

/// Public projection of older durable failures; retain private ledger diagnostics.
pub(crate) fn public_task_failure_reason(reason: &str) -> String {
    if reason.contains("Provider prompt dispatch failed: provider protocol `") {
        if let Some((_, operation)) = reason.split_once("provider protocol `") {
            if let Some((operation, _)) = operation.split_once('`') {
                return format!(
                    "{} Check the result before using Resume to retry, or Cancel.",
                    operation_failure_summary(operation)
                );
            }
        }
    }
    reason.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_durable_dispatch_reason_has_no_internal_protocol_text() {
        let reason = public_task_failure_reason("The provider could not confirm the request: Provider prompt dispatch failed: provider protocol `session_permissions` failed for run `private-run`: read failed. Check the result before using Resume to retry, or Cancel.");
        assert!(reason.contains("session permissions"));
        assert!(reason.contains("Resume"));
        assert!(!reason.contains("private-run"));
        assert!(!reason.contains("provider protocol"));
        assert_eq!(
            public_task_failure_reason("Owner must restore the service"),
            "Owner must restore the service"
        );
    }

    #[test]
    fn internal_dispatch_diagnostic_has_a_plain_owner_cause() {
        let reason = user_facing_dispatch_failure(&DaemonError::ProviderProtocol {
            provider_run_id: "private-run-id".into(),
            operation: "session_permissions",
            message: "Resource temporarily unavailable (os error 11)".into(),
        });
        assert!(reason.contains("session permissions"));
        assert!(!reason.contains("private-run-id"));
        assert!(!reason.contains("session_permissions"));
        assert!(!reason.contains("provider protocol"));
        assert!(is_internal_dispatch_diagnostic(
            "Provider prompt dispatch failed: detail"
        ));
        assert!(!is_internal_dispatch_diagnostic("The agent's own answer"));
    }
}
