//! A refused dispatch is recoverable without replacing the Room's runtime.
use crate::error::DaemonError;
use crate::session::EnvironmentLifecycle;
use crate::slice::ENVIRONMENT_USE_ADMISSION_EXPIRED;

pub(super) fn lifecycle_after_start_error(error: &DaemonError) -> EnvironmentLifecycle {
    if matches!(error, DaemonError::LocalTransport { operation: "browser_controller.route", message }
        if message == ENVIRONMENT_USE_ADMISSION_EXPIRED)
    {
        // No controller command ran. Retain the runtime and its Tab identities;
        // the normal health poll can finish startup when the slice queue drains.
        EnvironmentLifecycle::Degraded
    } else {
        EnvironmentLifecycle::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{CanonicalViewport, RoomEnvironment};

    #[test]
    fn queued_start_timeout_keeps_restart_recovery_available() {
        let mut room = RoomEnvironment::new(
            "room",
            "environment",
            CanonicalViewport::new(800, 600, 1, 800, 600).unwrap(),
        )
        .unwrap();
        room.start_runtime().unwrap();
        let tab = room
            .register_or_reconcile_tab("target", "https://app.invalid", "Todo")
            .unwrap();
        room.transition_to(EnvironmentLifecycle::Ready).unwrap();
        room.reconcile_after_kernel_restart();
        let error = DaemonError::LocalTransport {
            operation: "browser_controller.route",
            message: ENVIRONMENT_USE_ADMISSION_EXPIRED.into(),
        };
        // Startup completion ignores a redundant Degraded -> Degraded
        // transition after restart, just as it does in production.
        let _ = room.transition_to(lifecycle_after_start_error(&error));
        assert_eq!(room.snapshot().lifecycle, EnvironmentLifecycle::Degraded);
        // Normal health completion can recover the existing runtime and Tab.
        room.transition_to(EnvironmentLifecycle::Ready).unwrap();
        assert_eq!(room.snapshot().tabs[0].tab_id, tab);
    }

    #[test]
    fn actual_controller_start_failure_remains_failed() {
        for (operation, message) in [
            (
                "browser_controller.route",
                "browser_controller_scope_denied",
            ),
            ("browser_controller.route", "controller exited"),
            ("another.operation", ENVIRONMENT_USE_ADMISSION_EXPIRED),
        ] {
            let error = DaemonError::LocalTransport {
                operation,
                message: message.into(),
            };
            assert_eq!(
                lifecycle_after_start_error(&error),
                EnvironmentLifecycle::Failed
            );
        }
    }
}
