use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::KernelCommand;
use crate::runtime::command_latency::{log_command_completed, log_command_received, CommandTrace};
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
        let command_trace = CommandTrace::from_command(&command);
        log_command_received(&command_trace);
        let focus_refresh = focus_projection_refresh(&request);
        let caller_user_id = match authorize_session_membership(
            &self.runtime_state,
            &self.session_projection,
            &command,
            &request,
        )
        .await
        {
            Ok(caller_user_id) => caller_user_id,
            Err(error) => {
                let result = Err(error);
                log_command_completed(&command_trace, &result);
                return result;
            }
        };
        if let Some((session_id, attachment_id)) = terminal_poll_attachment(&request) {
            if let Err(error) = self
                .record_terminal_attachment_heartbeat(
                    session_id,
                    attachment_id,
                    crate::session::unix_epoch_ms(),
                )
                .await
            {
                let result = Err(error);
                log_command_completed(&command_trace, &result);
                return result;
            }
        }
        match self
            .dispatch_pre_lane(&command, &request, &caller_user_id)
            .await
        {
            Ok(Some(response)) => {
                let result = Ok(response);
                log_command_completed(&command_trace, &result);
                return result;
            }
            Ok(None) => {}
            Err(error) => {
                let result = Err(error);
                log_command_completed(&command_trace, &result);
                return result;
            }
        }

        let session_refresh = session_projection_refresh(&request);
        let result = self.dispatch_refresh_tracked(command, request).await;
        refresh_command_response_state(
            CommandResponseRefreshContext {
                app: &self.app,
                session_projection: &self.session_projection,
                agent_runtime_projection: &self.agent_runtime_projection,
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
        let result = self.redact_result_for_user(result, &caller_user_id);
        log_command_completed(&command_trace, &result);
        result
    }
}

fn terminal_poll_attachment(request: &LocalDaemonRequest) -> Option<(&str, &str)> {
    match request {
        LocalDaemonRequest::PumpTerminalOutput(request) => {
            Some((&request.session_id, &request.attachment_id))
        }
        LocalDaemonRequest::PollRuntimeNotices(request) => {
            Some((&request.session_id, &request.attachment_id))
        }
        _ => None,
    }
}
