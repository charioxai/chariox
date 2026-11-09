//! MP-08 / MP-11: project only existing, trusted policy codes to providers.
use crate::error::DaemonError;
pub(super) fn message(error: &DaemonError) -> String {
    match error {
        DaemonError::UserDomainRefused { reason } => {
            format!("{}: User-domain request refused", reason.code())
        }
        _ => error.to_string(),
    }
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
        let sentence = "agent is unavailable in this room";
        let forwarded = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: crate::runtime::room_tool_admission::REFUSED_CODE.to_owned(),
            message: sentence.to_owned(),
            retryable: false,
        };
        assert_eq!(message(&forwarded), sentence);
    }
}
