//! MP-08 / MP-10 / MP-11: isolate App admission from other pre-lane poll frames.
use std::{future::Future, pin::Pin};

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::KernelCommand;

use super::CommandRouter;

type AppPreLaneFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<LocalDaemonResponse>, DaemonError>> + Send + 'a>>;

impl CommandRouter {
    // Construct the App future outside the surrounding router's poll frame.
    // Authority stays in the same task and every existing handler keeps its order.
    #[inline(never)]
    pub(super) fn dispatch_app_pre_lane<'a>(
        &'a self,
        command: &'a KernelCommand,
        request: &'a LocalDaemonRequest,
        caller_user_id: &'a str,
    ) -> AppPreLaneFuture<'a> {
        Box::pin(self.dispatch_app_pre_lane_body(command, request, caller_user_id))
    }

    async fn dispatch_app_pre_lane_body(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
        caller_user_id: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        if let Some(response) = self
            .runtime_state
            .app_control()
            .publishers()
            .execute(&self.runtime_state, command, request)
            .await
        {
            return Ok(Some(response));
        }
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if let Some(response) = self
            .runtime_state
            .app_control()
            .installs()
            .execute(&self.runtime_state, command, request)
            .await
        {
            return Ok(Some(response));
        }
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if let Some(response) = self
            .runtime_state
            .execute_user_app_view_request(command, request)
            .await
        {
            return response.map(Some);
        }
        // Protocols 358–360: App routes and grants on a generator connection
        // are checked with that generator before the App lane stores them.
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if let Some(response) = self
            .dispatch_app_event_request(command, request, caller_user_id)
            .await?
        {
            return Ok(Some(response));
        }
        if let Some(response) = self
            .runtime_state
            .app_control()
            .execute(command, request)
            .await
        {
            return Ok(Some(response));
        }
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if let Some(response) = self
            .runtime_state
            .execute_app_control_request(command, request)
            .await
        {
            return Ok(Some(response));
        }
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if let Some(response) = Box::pin(
            self.runtime_state
                .execute_app_view_request(command, request),
        )
        .await
        {
            return response.map(Some);
        }
        Ok(None)
    }
}
