use crate::error::DaemonError;
use crate::runtime::browser_controller_file_transfer::{
    BrowserControllerDownloadCancellationResult, BrowserDownloadCancellation,
};
use crate::session::{agent_environment_actor_id, EnvironmentActionRequest};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomBrowserControllerResult,
};

use super::room_browser_controller::controller_route_error;
use super::{BrowserControllerActionExecution, KernelRuntimeState};

impl KernelRuntimeState {
    pub(crate) async fn cancel_browser_download_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        cancellation: BrowserDownloadCancellation,
    ) -> Result<
        BrowserControllerActionExecution<BrowserControllerDownloadCancellationResult>,
        DaemonError,
    > {
        let environment = self
            .reconcile_room_environment_actors(session_id, None)
            .map_err(|error| controller_route_error(&format!("{}: {error:?}", error.code())))?;
        let request = EnvironmentActionRequest::browser_download_cancellation(
            agent_environment_actor_id(agent_id),
            environment.runtime_generation,
        );
        self.execute_browser_mutation(session_id, request, None, async {
            let response = self
                .room_browser_controller_command(
                    session_id,
                    RoomBrowserControllerCommand::CancelDownload {
                        cancellation: cancellation.clone(),
                    },
                )
                .await?;
            let RoomBrowserControllerResult::DownloadCancellation {
                result: Some(result),
            } = response
            else {
                return Err(controller_route_error(
                    "missing controller download cancellation response",
                ));
            };
            result
                .validate(&cancellation)
                .map_err(|error| controller_route_error(&error))?;
            Ok(result)
        })
        .await
    }
}
