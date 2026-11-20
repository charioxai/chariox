//! A target that refuses a request because it is too old says which protocol it runs and which
//! one it needs, in one typed refusal, whoever asked (client, Cloud, peer or external agent).
use super::error::GatewayError;
use super::link::Link;
use super::self_hosted::status_of;

/// The oldest local protocol this gateway serves: the 10-14 release (1008). A kernel below it
/// cannot read what the gateway sends, and no retry changes that.
pub(crate) const MINIMUM_TARGET_PROTOCOL: u32 = 1008;

/// `refusal`, unless the target's protocol shows it is too old: then `UpdateNeeded`.
fn settle(name: &str, protocol: Option<u32>, refusal: GatewayError) -> GatewayError {
    match protocol {
        Some(kernel_protocol) if kernel_protocol < MINIMUM_TARGET_PROTOCOL => {
            GatewayError::UpdateNeeded {
                name: name.into(),
                kernel_protocol,
                needed_protocol: MINIMUM_TARGET_PROTOCOL,
            }
        }
        _ => refusal,
    }
}

/// Asks the target that refused for its protocol (every kernel answers `RelayStatus`) and
/// turns a refusal that blames its age into `UpdateNeeded`. A target that does not answer keeps
/// the refusal it gave.
pub(super) async fn after_refusal(name: &str, link: &Link, refusal: GatewayError) -> GatewayError {
    if !matches!(
        refusal,
        GatewayError::Incompatible { .. } | GatewayError::JoinRefused { .. }
    ) {
        return refusal;
    }
    match status_of(link).await {
        Ok(status) => settle(name, status.protocol, refusal),
        Err(_) => refusal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incompatible() -> GatewayError {
        GatewayError::Incompatible {
            name: "old".into(),
            detail: "it predates Room terminals".into(),
        }
    }

    #[test]
    fn a_target_below_the_minimum_is_refused_with_its_protocol_and_the_needed_one() {
        let error = settle("old", Some(MINIMUM_TARGET_PROTOCOL - 1), incompatible());
        assert_eq!(error.code(), "kernel_update_needed");
        assert!(!error.retryable());
        assert_eq!(
            error.message(),
            format!(
                "Kernel \"old\" runs protocol {} and needs {MINIMUM_TARGET_PROTOCOL}. Update the Chariox kernel on that machine, then try again.",
                MINIMUM_TARGET_PROTOCOL - 1
            )
        );
    }

    #[test]
    fn a_current_or_silent_target_keeps_its_own_refusal() {
        for protocol in [None, Some(MINIMUM_TARGET_PROTOCOL)] {
            assert_eq!(
                settle("old", protocol, incompatible()).code(),
                "kernel_incompatible"
            );
        }
    }
}
