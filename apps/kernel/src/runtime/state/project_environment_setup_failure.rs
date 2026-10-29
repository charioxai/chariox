//! MP-08 / MP-10 / MP-11: safe causal diagnostics for provider readiness.
use crate::error::DaemonError;

// Read only the derived enum name; stop before any diagnostic field is formatted.
fn error_kind(error: &DaemonError) -> String {
    struct Kind(String);
    impl std::fmt::Write for Kind {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            for c in value.chars() {
                if !(c.is_ascii_alphanumeric() || c == '_') || self.0.len() >= 64 {
                    return Err(std::fmt::Error);
                }
                self.0.push(c);
            }
            Ok(())
        }
    }
    use std::fmt::Write;
    let mut kind = Kind(String::new());
    let _ = write!(&mut kind, "{error:?}");
    kind.0
}

pub(super) fn provider_readiness_failure(error: &DaemonError) -> (&'static str, String) {
    let (cause, source) = match error {
        DaemonError::LocalTransport { operation, message } => {
            let cause = if message.contains(" is not authenticated;") {
                "provider_account_authentication_required"
            } else if message.contains("cannot start new work because") {
                "provider_account_usage_exhausted"
            } else if message.contains("provider runtime is not running") {
                "provider_runtime_not_running"
            } else if message.contains("remote-backed") {
                "provider_worker_context_required"
            } else if message.contains(" is busy") {
                "provider_agent_busy"
            } else if message.to_ascii_lowercase().contains("vault")
                && message.to_ascii_lowercase().contains("locked")
            {
                "provider_credential_vault_locked"
            } else if *operation == "get_provider_auth_status" {
                "provider_auth_status_failed"
            } else {
                "provider_context_error"
            };
            (cause.to_string(), *operation)
        }
        DaemonError::ManagedContext {
            code, operation, ..
        } => (format!("managed_context:{code}"), *operation),
        _ => (
            format!("kernel:{}", error_kind(error)),
            "provider readiness",
        ),
    };
    // Operation names are kernel static metadata; never use Display or Debug fields.
    let source: String = source.chars().take(80).collect();
    (
        "worker_provider_context_unavailable",
        format!(
            "[cause:{cause}] provider readiness unavailable [error:{}; source:{source}]",
            error_kind(error)
        ),
    )
}

pub(super) fn record_provider_readiness_failure(
    store: &super::ProjectEnvironmentSetupStore,
    execution: &super::SetupExecution,
    attempt: u32,
    error: &DaemonError,
) {
    let (code, message) = provider_readiness_failure(error);
    crate::logging::warn_with_fields(
        "daemon.project_environment_setup",
        "provider readiness rejected Environment setup",
        serde_json::json!({
            "operation_id": execution.operation_id,
            "session_id": execution.session_id,
            "execution_session_id": execution.execution_session_id,
            "execution_agent_id": execution.execution_agent_id,
            "cause": message,
        }),
    );
    store.mark_failed(&execution.operation_id, attempt, code, &message);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn envp03_setup_preserves_account_cause_without_diagnostic_values() {
        let error = DaemonError::LocalTransport {operation:"ensure prompt provider run for agent",message:"selected claude account `PRIVATE-CONTROLLER-DATA` is not authenticated; authenticate it in Provider Accounts before starting new work".into()};
        let (code, message) = provider_readiness_failure(&error);
        assert_eq!(code, "worker_provider_context_unavailable");
        assert!(message.contains("cause:provider_account_authentication_required"));
        assert!(message.contains("source:ensure prompt provider run for agent"));
        assert!(!message.contains("PRIVATE-CONTROLLER-DATA"));
    }
    #[test]
    fn envp03_setup_preserves_native_status_failure_without_provider_output() {
        let error = DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: "PRIVATE-CONTROLLER-DATA".into(),
        };
        let (_, message) = provider_readiness_failure(&error);
        assert!(message.contains("cause:provider_auth_status_failed"));
        assert!(message.contains("source:get_provider_auth_status"));
        assert!(!message.contains("PRIVATE-CONTROLLER-DATA"));
    }
    #[test]
    fn envp03_setup_preserves_managed_context_reason_without_values() {
        let error = DaemonError::ManagedContext {
            code: "profile_scope_missing",
            operation: "bind provider context",
            message: "PRIVATE-CONTROLLER-DATA".into(),
            retryable: false,
        };
        let (_, message) = provider_readiness_failure(&error);
        assert!(message.contains("cause:managed_context:profile_scope_missing"));
        assert!(!message.contains("PRIVATE-CONTROLLER-DATA"));
    }
}
