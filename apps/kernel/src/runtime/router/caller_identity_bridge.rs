use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::local::LocalDaemonResponse;
use crate::runtime::command::{KernelCaller, KernelCommandSource};
use crate::runtime::response_redaction::redact_response_for_user;

impl CommandRouter {
    pub(crate) async fn local_command_caller(&self, source: KernelCommandSource) -> KernelCaller {
        let mut caller = KernelCaller::for_source(&source);
        let cloud_profile = self.config_projection.snapshot().cloud_relay;
        if let Some(profile) = cloud_profile {
            caller.user_id = Some(profile.user_id);
            caller.client_id = profile.client_id;
            caller.machine_id = profile.machine_id;
            caller.realm_id = Some(profile.realm_id);
        }
        caller
    }

    pub(super) fn redact_result_for_user(
        &self,
        result: Result<LocalDaemonResponse, DaemonError>,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        result.and_then(|response| {
            redact_response_for_user(
                response,
                caller_user_id,
                &self.provider_run_projection,
                None,
            )
        })
    }

    pub(super) fn redact_workflow_result_for_user(
        &self,
        result: Result<LocalDaemonResponse, DaemonError>,
        caller_user_id: &str,
        request: &LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = match request {
            LocalDaemonRequest::ListWorkflowRuns(request) => Some(request.session_id.as_str()),
            LocalDaemonRequest::GetWorkflowRun(request) => Some(request.session_id.as_str()),
            _ => None,
        };
        let workflow_run_context =
            session_id.and_then(|session_id| self.session_projection.get_shared(session_id));
        result.and_then(|response| {
            redact_response_for_user(
                response,
                caller_user_id,
                &self.provider_run_projection,
                workflow_run_context.as_deref(),
            )
        })
    }
}
