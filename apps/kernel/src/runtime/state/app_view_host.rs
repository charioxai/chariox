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
            match Box::pin(
                self.state
                    .room_browser_controller_command(self.session, Command::AppView { request }),
            )
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

pub(super) struct KernelBrowserAppViewHost<'a> {
    pub state: &'a KernelRuntimeState,
    pub owner: &'a str,
    pub generation: Option<u64>,
}

impl AppViewHost for KernelBrowserAppViewHost<'_> {
    fn command(
        &self,
        request: BrowserAppViewRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Value>, DaemonError>> + Send + '_>> {
        Box::pin(async move {
            Box::pin(self.state.kernel_browser_app_view(
                self.owner.into(),
                request,
                self.generation,
            ))
            .await
            .map(Some)
        })
    }
}
