use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::KernelCommand;
use crate::runtime::command_response_refresh::{
    refresh_command_response_state, CommandResponseRefreshContext,
};
use crate::runtime::session_membership::authorize_session_membership;
use crate::runtime::session_projection_refresh::{
    focus_projection_refresh, session_projection_refresh,
};

use super::CommandRouter;

impl CommandRouter {
    pub(crate) async fn dispatch(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let focus_refresh = focus_projection_refresh(&request);
        let caller_user_id =
            authorize_session_membership(&self.app, &self.session_projection, &command, &request)
                .await?;
        if let Some(response) = self
            .dispatch_pre_lane(&command, &request, &caller_user_id)
            .await?
        {
            return Ok(response);
        }

        let session_refresh = session_projection_refresh(&request);
        let result = self.dispatch_refresh_tracked(command, request).await;
        refresh_command_response_state(
            CommandResponseRefreshContext {
                app: &self.app,
                session_projection: &self.session_projection,
                agent_runtime_projection: &self.agent_runtime_projection,
                history_projection: &self.history_projection,
                focus_projection: &self.focus_projection,
                provider_process_projection: &self.provider_process_projection,
                provider_launch_pending: &self.provider_launch_pending,
                provider_run_projection: &self.provider_run_projection,
                agent_runtime: &self.agent_runtime,
                workflow_runtime: &self.workflow_runtime,
            },
            session_refresh,
            focus_refresh,
            &result,
        )
        .await;
        self.redact_result_for_user(result, &caller_user_id)
    }
}
