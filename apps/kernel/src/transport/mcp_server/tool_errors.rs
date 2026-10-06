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
}
