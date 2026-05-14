//! Terminal-facing session-store request adapters.

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, PollRuntimeNoticesRequest, ResizeTerminalRequest, SendTerminalInputRequest,
};

use super::super::projection_policy::SessionProjectionAction;
use super::SessionRuntimeStore;

impl SessionRuntimeStore {
    pub(in crate::runtime::session_actor) async fn resize_terminal(
        &self,
        request: ResizeTerminalRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .resize_terminal(&request.session_id, request.cols, request.rows)
            .await
            .map(|()| LocalDaemonResponse::TerminalResized {
                session_id: request.session_id,
                cols: request.cols,
                rows: request.rows,
            });
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn send_terminal_input(
        &self,
        request: SendTerminalInputRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = self
            .state
            .send_terminal_input(
                &request.session_id,
                &request.attachment_id,
                request.provider_run_id.as_deref(),
                &request.data_base64,
            )
            .await
            .map(|byte_count| LocalDaemonResponse::TerminalInputSent {
                session_id: request.session_id,
                attachment_id: request.attachment_id,
                byte_count,
            });
        self.with_session_projection_action_result(result).await
    }

    pub(in crate::runtime::session_actor) async fn poll_runtime_notices(
        &self,
        request: PollRuntimeNoticesRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        let result = match self
            .state
            .ensure_attachment_in_session(&request.session_id, &request.attachment_id)
            .await
        {
            Ok(()) => Ok(LocalDaemonResponse::RuntimeNotices {
                notices: self
                    .state
                    .drain_notice_records(&request.session_id, &request.attachment_id)
                    .await,
            }),
            Err(error) => Err(error),
        };
        self.with_session_projection_action_result(result).await
    }
}
