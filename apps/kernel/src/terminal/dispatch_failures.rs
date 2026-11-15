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
        DaemonError::ProviderRefusal { message } => {
            return crate::provider::TurnOutcome::classify("", message).notice();
        }
        DaemonError::LocalTransport { message, .. } => {
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
        DaemonError::ProviderRefusal { message } => {
            Some(crate::provider::TurnOutcome::classify(adapter_key, message))
        }
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
    let lower = reason.to_ascii_lowercase();
    if lower.contains("user_domain_not_granted")
        || lower.contains("chariox.load_kernel_browser")
        || lower.contains("grant state changed")
    {
        return "Browser access ended. Allow browser access to continue.".into();
    }
    if ["retained obligations", "attributed result", "condition_met"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return "Waiting for the result.".into();
    }
    if lower.contains("retained instructions") {
        return "This task needs your help to continue.".into();
    }
    if lower.contains("repeated invalid turn end") {
        return "Its turn ended before its work was done. Resume to continue, or Cancel.".into();
    }
    if lower.contains("unconfirmed delivery") {
        return "A message to it may not have arrived. Check its last turn, then Resume or Cancel."
            .into();
    }
    if lower.contains("owner must reconcile") {
        return "Its work was interrupted. Check its last turn, then Resume or Cancel.".into();
    }
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
    fn owner_task_failure_uses_plain_browser_and_waiting_words() {
        assert_eq!(
            public_task_failure_reason(
                "chariox.load_kernel_browser failed: user_domain_not_granted"
            ),
            "Browser access ended. Allow browser access to continue."
        );
        assert_eq!(
            public_task_failure_reason(
                "Waiting for an attributed result and condition_met receipt; retained obligations"
            ),
            "Waiting for the result."
        );
    }

    /// Live (orch10-cards, PCD-01 F5): after a restart the person's card read "agent-1: Repeated
    /// invalid turn end; owner must resume or cancel". A blocked task's card says in plain words
    /// what happened and what Resume and Cancel do.
    #[test]
    fn a_blocked_task_card_reads_in_plain_words() {
        for (reason, plain) in [
            (
                "Repeated invalid turn end; owner must resume or cancel",
                "Its turn ended before its work was done. Resume to continue, or Cancel.",
            ),
            (
                "Unconfirmed delivery 7 prevents this task's wake; owner must reconcile the original attempt or cancel, then explicitly resume",
                "A message to it may not have arrived. Check its last turn, then Resume or Cancel.",
            ),
            (
                "Unconfirmed delivery 7: owner must reconcile the original attempt",
                "A message to it may not have arrived. Check its last turn, then Resume or Cancel.",
            ),
            (
                "Recovered legacy dispatch obligation; owner must reconcile its exact resource and resume or cancel",
                "Its work was interrupted. Check its last turn, then Resume or Cancel.",
            ),
        ] {
            assert_eq!(public_task_failure_reason(reason), plain);
        }
    }

    #[test]
    fn ea2_75_refused_dispatch_errors_use_the_same_calm_recovery_text() {
        let error = DaemonError::ProviderRefusal {
            message: "Codex error [server_overloaded]: Selected model is at capacity. Please try a different model.".into(),
        };
        assert_eq!(
            user_facing_dispatch_failure(&error),
            "The provider is at capacity. Try again in a moment or choose another model."
        );
    }

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
