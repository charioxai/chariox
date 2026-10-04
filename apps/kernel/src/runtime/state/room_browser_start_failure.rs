//! Recoverable startup downtime retains the Room runtime for its health poll.
use crate::error::DaemonError;
use crate::session::EnvironmentLifecycle;
use crate::slice::ENVIRONMENT_USE_ADMISSION_EXPIRED;
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};

const CAPTURE_RECOVERY_WINDOW: Duration = Duration::from_secs(120);

/// Startup and health observations share one finite capture-recovery window.
#[derive(Default)]
pub(super) struct RoomBrowserStartRecovery {
    deadlines: Mutex<BTreeMap<String, Instant>>,
}

impl RoomBrowserStartRecovery {
    pub(super) fn after_start_error(
        &self,
        session: &str,
        error: &DaemonError,
        now: Instant,
    ) -> EnvironmentLifecycle {
        if matches!(error, DaemonError::LocalTransport { operation: "browser_controller.route", message }
            if message == ENVIRONMENT_USE_ADMISSION_EXPIRED)
        {
            // Admission refused dispatch; preserve the existing recovery path.
            return EnvironmentLifecycle::Degraded;
        }
        self.viewport_failure(session, error, now)
            .unwrap_or(EnvironmentLifecycle::Failed)
    }

    pub(super) fn viewport_failure(
        &self,
        session: &str,
        error: &DaemonError,
        now: Instant,
    ) -> Option<EnvironmentLifecycle> {
        let controller_transport = matches!(
            error,
            DaemonError::LocalTransport {
                operation: "browser_controller.route",
                ..
            } | DaemonError::RelayTransport {
                operation: "read relay peer response",
                ..
            }
        );
        if !controller_transport
            || !error.to_string().contains(
                &crate::runtime::browser_controller_process::controller_error_marker(
                    "viewport_apply_failed",
                ),
            )
        {
            return None;
        }
        let mut deadlines = self
            .deadlines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deadline = deadlines
            .entry(session.into())
            .or_insert(now + CAPTURE_RECOVERY_WINDOW);
        Some(if now < *deadline {
            EnvironmentLifecycle::Degraded
        } else {
            EnvironmentLifecycle::Failed
        })
    }

    pub(super) fn reset(&self, session: &str) {
        self.deadlines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session);
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
        let recovery = RoomBrowserStartRecovery::default();
        let _ = room.transition_to(recovery.after_start_error("room", &error, Instant::now()));
        assert_eq!(room.snapshot().lifecycle, EnvironmentLifecycle::Degraded);
        // Normal health completion can recover the existing runtime and Tab.
        room.transition_to(EnvironmentLifecycle::Ready).unwrap();
        assert_eq!(room.snapshot().tabs[0].tab_id, tab);
    }

    #[test]
    fn canonical_capture_downtime_keeps_cold_room_recovery_available() {
        let mut room = RoomEnvironment::new(
            "room",
            "environment",
            CanonicalViewport::new(390, 844, 1, 390, 844).unwrap(),
        )
        .unwrap();
        room.start_runtime().unwrap();
        let tab = room
            .register_or_reconcile_tab("target", "https://app.invalid", "Todo")
            .unwrap();
        let recovery = RoomBrowserStartRecovery::default();
        for error in [local_capture_error(), relay_capture_error()] {
            let _ = room.transition_to(recovery.after_start_error("room", &error, Instant::now()));
            assert_eq!(room.snapshot().lifecycle, EnvironmentLifecycle::Degraded);
            // The normal verified health completion recovers the same runtime.
            room.transition_to(EnvironmentLifecycle::Ready).unwrap();
            assert_eq!(room.snapshot().tabs[0].tab_id, tab);
            assert_eq!(
                room.snapshot().focused_tab_id.as_deref(),
                Some(tab.as_str())
            );
        }
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
            (
                "another.operation",
                "browser controller `browser.reconcile` failed with viewport_apply_failed: failure",
            ),
        ] {
            let error = DaemonError::LocalTransport {
                operation,
                message: message.into(),
            };
            assert_eq!(
                RoomBrowserStartRecovery::default().after_start_error(
                    "room",
                    &error,
                    Instant::now()
                ),
                EnvironmentLifecycle::Failed
            );
        }
    }

    fn local_capture_error() -> DaemonError {
        DaemonError::LocalTransport {
            operation: "browser_controller.route",
            message: "browser controller `browser.reconcile` failed with viewport_apply_failed: canonical physical display apply failed".into(),
        }
    }

    fn relay_capture_error() -> DaemonError {
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "transport_error".into(),
            message: "local transport `browser_controller.route` failed: browser controller `browser.reconcile` failed with viewport_apply_failed: canonical physical display apply failed".into(),
            retryable: true,
        }
    }

    #[test]
    fn start_and_health_share_a_deadline_for_local_and_relay_capture_failures() {
        let recovery = RoomBrowserStartRecovery::default();
        let now = Instant::now();
        assert_eq!(
            recovery.after_start_error("room", &local_capture_error(), now),
            EnvironmentLifecycle::Degraded
        );
        assert_eq!(
            recovery.viewport_failure(
                "room",
                &relay_capture_error(),
                now + CAPTURE_RECOVERY_WINDOW - Duration::from_secs(1)
            ),
            Some(EnvironmentLifecycle::Degraded)
        );
        assert_eq!(
            recovery.viewport_failure(
                "room",
                &local_capture_error(),
                now + CAPTURE_RECOVERY_WINDOW
            ),
            Some(EnvironmentLifecycle::Failed)
        );
        assert_eq!(
            recovery.after_start_error(
                "room",
                &relay_capture_error(),
                now + CAPTURE_RECOVERY_WINDOW + Duration::from_secs(60)
            ),
            EnvironmentLifecycle::Failed
        );
        // Another Room's recovery is independent.
        assert_eq!(
            recovery.after_start_error(
                "other",
                &relay_capture_error(),
                now + CAPTURE_RECOVERY_WINDOW
            ),
            EnvironmentLifecycle::Degraded
        );
    }

    #[test]
    fn success_or_explicit_restart_rearms_the_bounded_window() {
        let recovery = RoomBrowserStartRecovery::default();
        let now = Instant::now();
        assert_eq!(
            recovery.after_start_error("room", &relay_capture_error(), now),
            EnvironmentLifecycle::Degraded
        );
        let expired = now + CAPTURE_RECOVERY_WINDOW;
        assert_eq!(
            recovery.after_start_error("room", &relay_capture_error(), expired),
            EnvironmentLifecycle::Failed
        );
        recovery.reset("room");
        assert_eq!(
            recovery.after_start_error("room", &local_capture_error(), expired),
            EnvironmentLifecycle::Degraded
        );
        assert_eq!(
            recovery.after_start_error(
                "room",
                &local_capture_error(),
                expired + CAPTURE_RECOVERY_WINDOW
            ),
            EnvironmentLifecycle::Failed
        );
    }
}
