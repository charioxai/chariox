//! Presentation seam. Host commands carry kernel-verified bytes, never caller
//! authority. Permission checks and App calls remain in the runtime services.
use super::KernelRuntimeState;
use crate::{error::DaemonError, runtime::browser_controller_app_view::BrowserAppViewRequest};
use serde_json::Value;
use std::{future::Future, pin::Pin};

pub(super) trait AppViewHost {
    fn command(
        &self,
        request: BrowserAppViewRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Value>, DaemonError>> + Send + '_>>;
}

pub(super) struct RoomAppViewHost<'a> {
    pub state: &'a KernelRuntimeState,
    pub session: &'a str,
}

impl AppViewHost for RoomAppViewHost<'_> {
    fn command(
        &self,
        request: BrowserAppViewRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Value>, DaemonError>> + Send + '_>> {
        Box::pin(async move {
            use crate::transport::room_browser_controller::{
                RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
            };
            match self
                .state
                .room_browser_controller_command(self.session, Command::AppView { request })
                .await?
            {
                Response::AppView {
                    result: Some(value),
                } => Ok(Some(value)),
                _ => Ok(None),
            }
        })
    }
}

pub(super) struct ClientNativeAppViewHost<'a> {
    pub state: &'a KernelRuntimeState,
    pub owner: &'a str,
    pub binding: crate::runtime::app_views::AppViewBinding,
}

impl AppViewHost for ClientNativeAppViewHost<'_> {
    fn command(
        &self,
        request: BrowserAppViewRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Value>, DaemonError>> + Send + '_>> {
        Box::pin(async move {
            let BrowserAppViewRequest::Open { origin_label, .. } = request else {
                return Err(super::app_view_runtime::open_error(
                    "Unsupported client-native host command",
                ));
            };
            let view = self
                .state
                .app_control()
                .user_views()
                .open(self.owner, self.binding.clone(), &origin_label)
                .map_err(|_| {
                    super::app_view_runtime::open_error("Too many open user-domain App views")
                })?;
            Ok(Some(
                serde_json::json!({"target_id": view.view_id, "origin": view.origin}),
            ))
        })
    }
}
