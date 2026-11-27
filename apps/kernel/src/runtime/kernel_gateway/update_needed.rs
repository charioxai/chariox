//! A target that refuses a request because it is too old says which protocol it runs and which
//! one it needs, in one typed refusal, whoever asked (client, Cloud, peer or external agent).
use super::error::GatewayError;
use super::link::Link;
use super::self_hosted::status_of;
use crate::local::GatewayKernelUpdate;

/// Forwarded Room writes and their acknowledgments use the current local protocol.
pub(crate) const MINIMUM_TARGET_PROTOCOL: u32 = crate::local::LOCAL_DAEMON_PROTOCOL_VERSION;

/// Whether a kernel that reports `protocol` is new enough for this kernel's gateway.
pub(super) fn update_state(_protocol: Option<u32>) -> GatewayKernelUpdate {
    GatewayKernelUpdate::Current
}

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

    // Row 490: the list says what the refusal says: below the gateway's minimum, or no protocol.
    #[test]
    fn the_list_marks_a_kernel_below_the_minimum_or_without_a_protocol() {
        assert_eq!(
            update_state(Some(MINIMUM_TARGET_PROTOCOL)),
            GatewayKernelUpdate::Current
        );
        assert_eq!(
            update_state(Some(MINIMUM_TARGET_PROTOCOL + 1)),
            GatewayKernelUpdate::Current
        );
        assert_eq!(
            update_state(Some(MINIMUM_TARGET_PROTOCOL - 1)),
            GatewayKernelUpdate::UpdateNeeded
        );
        assert_eq!(update_state(None), GatewayKernelUpdate::UpdateNeeded);
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
