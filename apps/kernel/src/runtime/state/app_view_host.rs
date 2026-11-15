//! Presentation seam. Host commands carry kernel-verified bytes, never caller
//! authority. Permission checks and App calls remain in the runtime services.
use super::KernelRuntimeState;
use crate::{error::DaemonError, runtime::browser_controller_app_view::BrowserAppViewRequest};
use serde_json::Value;
use std::{future::Future, pin::Pin};

/// A kernel started with this set to `1` refuses to project an App into a kernel browser, so a drill proves a
/// client reached every App's view natively: the person's window never needs the projection.
const REFUSE_PROJECTED_ENV: &str = "CHARIOX_REFUSE_PROJECTED_APP_VIEWS";

/// Every App page a kernel browser would show passes here as an `Open`, whether a Room's Tab or a user-domain one.
fn projected_open(route: &str, request: &BrowserAppViewRequest) -> Result<(), DaemonError> {
    let BrowserAppViewRequest::Open {
        installation_id, ..
    } = request
    else {
        return Ok(());
    };
    if std::env::var(REFUSE_PROJECTED_ENV).as_deref() == Ok("1") {
        tracing::warn!(route, installation = %installation_id, "refused: App view projected into a kernel browser");
        return Err(super::app_view_runtime::open_error(
            "This kernel does not project App views into its browser. Open the App in your client's own window.",
        ));
    }
    tracing::info!(route, installation = %installation_id, "App view projected into a kernel browser");
    Ok(())
}

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
            projected_open("room", &request)?;
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
            projected_open("user", &request)?;
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
