//! Provider-facing errors start with guidance. Transport details stay in the kernel log.
use crate::error::DaemonError;
pub(crate) fn message(error: &DaemonError) -> String {
    match error {
        DaemonError::UserDomainRefused { reason } => {
            format!("{}: User-domain request refused", reason.code())
        }
        DaemonError::LocalTransport { operation, message } => {
            log_transport_failure(operation, None);
            message.clone()
        }
        DaemonError::RelayTransport {
            operation,
            code,
            message,
            retryable,
        } => {
            log_transport_failure(operation, Some((code.as_str(), *retryable)));
            message.clone()
        }
        _ => error.to_string(),
    }
}
fn log_transport_failure(operation: &str, relay: Option<(&str, bool)>) {
    crate::logging::warn_with_fields(
        "daemon.runtime_mcp",
        "runtime MCP request failed",
        serde_json::json!({
            "operation": operation,
            "relay_code": relay.map(|(code, _)| code),
            "retryable": relay.map(|(_, retryable)| retryable),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp11_mcp_policy_refusal_retains_its_public_code() {
        let error = DaemonError::UserDomainRefused {
            reason: crate::error::UserDomainRefusalReason::NotGranted,
        };
        assert_eq!(
            message(&error),
            "user_domain_not_granted: User-domain request refused"
        );
    }

    /// A refusal that crossed the relay from the kernel that holds the room still
    /// reads as the sentence that kernel wrote.
    #[test]
    fn mp11_forwarded_room_refusal_reaches_the_provider_as_its_sentence() {
        use crate::runtime::room_tool_admission::{REFUSED_CODE, RETRY_CODE};
        let sentence = "agent is unavailable in this room";
        for (code, retryable) in [(REFUSED_CODE, false), (RETRY_CODE, true)] {
            let forwarded = DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: code.to_owned(),
                message: sentence.to_owned(),
                retryable,
            };
            assert_eq!(message(&forwarded), sentence);
        }
    }

    #[test]
    fn room_command_refusal_starts_with_its_guidance() {
        let guidance = "Unknown room command. Use chariox.commands.list to see available commands.";
        let error = DaemonError::LocalTransport {
            operation: "runtime_tool_meta.run_command",
            message: guidance.into(),
        };
        assert_eq!(message(&error), guidance);
    }

    const STALE_ELEMENT: &str =
        "browser controller `browser.action` failed with stale_element_reference: the element is stale";

    #[test]
    fn controller_refusal_reaches_the_agent_as_one_bare_sentence() {
        let received = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "browser_controller_failed".to_string(),
            message: STALE_ELEMENT.to_string(),
            retryable: false,
        };
        assert_eq!(message(&received), STALE_ELEMENT);
        let local = DaemonError::LocalTransport {
            operation: "browser_controller.route",
            message: STALE_ELEMENT.to_string(),
        };
        assert_eq!(message(&local), STALE_ELEMENT);
        let wrapped = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "transport_error".to_string(),
            message: STALE_ELEMENT.to_string(),
            retryable: true,
        };
        assert_eq!(message(&wrapped), STALE_ELEMENT);
    }
}
