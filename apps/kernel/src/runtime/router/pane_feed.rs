//! The pane feed is read through the requests a client of the Room may make, so it shows
//! exactly what that caller may see.
use crate::error::DaemonError;
use crate::local::{
    GetPaneFeedRequest, GetPersonInboxRequest, GetSessionStateRequest, LocalDaemonRequest,
    LocalDaemonResponse, ReportPaneSettingsRequest,
};
use crate::runtime::command::{command_caller_user_id, KernelCommand};
use crate::runtime::pane_feed::{project_pane_feed, PaneFeedSources};

use super::CommandRouter;

impl CommandRouter {
    pub(super) async fn get_pane_feed(
        &self,
        command: &KernelCommand,
        request: GetPaneFeedRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let state = self
            .session_view_component(
                command,
                "pane-state",
                LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                    session_id: request.session_id.clone(),
                }),
            )
            .await?;
        let LocalDaemonResponse::SessionState {
            session,
            agent_activity,
            ..
        } = state
        else {
            return Err(unexpected_response());
        };
        let inbox = self
            .session_view_component(
                command,
                "pane-inbox",
                LocalDaemonRequest::GetPersonInbox(GetPersonInboxRequest),
            )
            .await?;
        let LocalDaemonResponse::PersonInbox { cards, .. } = inbox else {
            return Err(unexpected_response());
        };
        let cloud_origin = self
            .runtime_state
            .config_snapshot()
            .await
            .cloud_relay
            .and_then(|profile| {
                crate::transport::relay_client::paired_cloud_origin(&profile.api_url)
            });
        let feed = project_pane_feed(
            &PaneFeedSources {
                session: &session,
                activity: &agent_activity,
                inbox: &cards,
                cloud_origin: cloud_origin.as_deref(),
            },
            &request.agent_id,
        )?;
        Ok(LocalDaemonResponse::PaneFeed { feed })
    }

    pub(super) async fn report_pane_settings(
        &self,
        command: &KernelCommand,
        request: ReportPaneSettingsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let feed_request = GetPaneFeedRequest {
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
        };
        self.runtime_state
            .report_pane_settings(request, &command_caller_user_id(command))
            .await?;
        self.get_pane_feed(command, feed_request).await
    }
}

fn unexpected_response() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "read pane feed",
        message: "the kernel answered a pane feed read with another response".into(),
    }
}
