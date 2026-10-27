//! Failure budgets for App restoration and registered-view polling.
use crate::{
    error::DaemonError,
    local::AppRequestErrorCode,
    runtime::{
        app_views::{AppViewBinding, AppViews},
        browser_controller_process::controller_error_marker,
    },
};
use std::time::Duration;
use tokio::time::Instant;

pub(super) const MAX_POLL_FAILURES: u32 = 20;
const VIEWPORT_DOWNTIME_GRACE: Duration = Duration::from_secs(120);

pub(super) enum ColdAppRestoreError {
    Busy,
    ViewportUnavailable,
    Failed(AppRequestErrorCode),
}

impl From<AppRequestErrorCode> for ColdAppRestoreError {
    fn from(code: AppRequestErrorCode) -> Self {
        Self::Failed(code)
    }
}

fn viewport_unavailable(message: &str) -> bool {
    message.contains(&controller_error_marker("viewport_apply_failed"))
}

pub(super) fn cold_restore_error(error: DaemonError) -> ColdAppRestoreError {
    let message = error.to_string();
    if crate::runtime::app_views::slice_busy(&message) || super::browser_recovery_downtime(&message)
    {
        ColdAppRestoreError::Busy
    } else if viewport_unavailable(&message) {
        ColdAppRestoreError::ViewportUnavailable
    } else {
        ColdAppRestoreError::Failed(AppRequestErrorCode::Conflict)
    }
}

#[derive(Default)]
pub(super) struct AppViewRecovery {
    viewport_deadline: Option<Instant>,
    poll_failures: u32,
}

impl AppViewRecovery {
    // A cold or resized stream may need to warm, but permanent display failures
    // must eventually spend the normal budgets. Restore and poll share a window.
    fn defer_viewport_failure(&mut self) -> bool {
        let now = Instant::now();
        let deadline = self
            .viewport_deadline
            .get_or_insert(now + VIEWPORT_DOWNTIME_GRACE);
        now < *deadline
    }

    pub(super) fn restore_finished(
        &mut self,
        views: &AppViews,
        session: &str,
        binding: &AppViewBinding,
        result: Result<(), ColdAppRestoreError>,
    ) {
        match result {
            Ok(())
            | Err(ColdAppRestoreError::Failed(
                AppRequestErrorCode::NotFound | AppRequestErrorCode::LimitExceeded,
            )) => views.finish_cold_start_view(session, binding),
            Err(ColdAppRestoreError::Busy) => {}
            Err(ColdAppRestoreError::ViewportUnavailable) if self.defer_viewport_failure() => {}
            Err(_) => views.fail_cold_start_view(session, binding),
        }
    }

    pub(super) fn poll_failed(&mut self, views: &AppViews, session: &str, error: &DaemonError) {
        let message = error.to_string();
        if crate::runtime::app_views::slice_busy(&message)
            || (viewport_unavailable(&message) && self.defer_viewport_failure())
        {
            return;
        }
        self.poll_failures += 1;
        if self.poll_failures >= MAX_POLL_FAILURES {
            views.forget_session(session);
        }
    }

    pub(super) fn poll_succeeded(&mut self) {
        self.poll_failures = 0;
        self.viewport_deadline = None;
    }
}
