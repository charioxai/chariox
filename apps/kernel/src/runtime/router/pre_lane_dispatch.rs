use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::KernelCommand;
use crate::runtime::daemon_health_projection::execute_daemon_health_request;
use crate::runtime::event_catalog_control::execute_event_catalog_request_with_client;
use crate::runtime::provider_run_control::projected_provider_run_response;
use crate::runtime::resource_telemetry::execute_kernel_resource_telemetry_request;
use crate::runtime::session_read_control::{
    projected_session_inspection_response, projected_session_read_response,
};
use crate::runtime::workflow_actor::is_workflow_command;

use super::CommandRouter;

impl CommandRouter {
    pub(super) async fn dispatch_pre_lane(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
        caller_user_id: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        if let Some(actor) = command.caller.metaagent_id.as_deref() {
            self.runtime_state
                .authorize_room_provider_epoch(Some(actor), command.provider_run_id.as_deref())?;
            self.runtime_state
                .authorize_room_agent_request(actor, request)?;
        }
        if let Some(response) = self
            .dispatch_app_pre_lane(command, request, caller_user_id)
            .await?
        {
            return Ok(Some(response));
        }
        if matches!(
            request,
            LocalDaemonRequest::PrepareBrowserImport(_)
                | LocalDaemonRequest::ApproveBrowserImport(_)
                | LocalDaemonRequest::ClaimBrowserImportSource(_)
                | LocalDaemonRequest::AuthorizeBrowserImportSource(_)
                | LocalDaemonRequest::CancelBrowserImport(_)
        ) {
            return self
                .runtime_state
                .execute_browser_import_consent(command, request)
                .await
                .map(Some);
        }
        if let Some(response) = projected_session_read_response(
            &self.runtime_state,
            &self.session_projection,
            &self.provider_run_projection,
            &self.provider_launch_pending,
            request,
            caller_user_id,
        )
        .await
        {
            return response.map(Some);
        }
        if let LocalDaemonRequest::RemoveEventConnection(removal) = request {
            let _connection_guard = self
                .event_connection_lanes
                .lock(caller_user_id, &removal.connection_id)
                .await;
            return execute_event_catalog_request_with_client(
                &self.runtime_state,
                &self.config_projection,
                &self.aegs_management_http_client,
                caller_user_id,
                request.clone(),
            )
            .await
            .map(Some);
        }
        let managed_connection_id = match request {
            LocalDaemonRequest::RefreshEventConnection(request) => {
                Some(request.connection_id.clone())
            }
            LocalDaemonRequest::TestEventConnection(request) => Some(request.connection_id.clone()),
            LocalDaemonRequest::ReconnectEventConnection(request) => {
                Some(request.connection_id.clone())
            }
            LocalDaemonRequest::ObserveEventConnectionAuthorization(request) => self
                .runtime_state
                .event_connection_registry()
                .authorization(caller_user_id, &request.authorization_id)?
                .and_then(|authorization| authorization.connection_id),
            _ => None,
        };
        if let Some(connection_id) = managed_connection_id {
            let _connection_guard = self
                .event_connection_lanes
                .lock(caller_user_id, &connection_id)
                .await;
            return execute_event_catalog_request_with_client(
                &self.runtime_state,
                &self.config_projection,
                &self.aegs_management_http_client,
                caller_user_id,
                request.clone(),
            )
            .await
            .map(Some);
        }
        // MP-08 / MP-10 / MP-11: poll only the selected request handler.
        // A missing provider projection still falls through to ordinary dispatch.
        if let Some(handler) = self.select_pre_lane_request(request, caller_user_id) {
            if let Some(response) = handler.await? {
                return Ok(Some(response));
            }
        }
        if let Some(response) = projected_session_inspection_response(
            &self.session_projection,
            request,
            caller_user_id,
            command
                .caller
                .metaagent_id
                .as_deref()
                .filter(|_| !self.runtime_state.room_agent_tools_enabled()),
        ) {
            return response.map(Some);
        }
        if let LocalDaemonRequest::PumpTerminalOutput(request) = request {
            if let Some(response) = self.terminal_output_executor.projected_response(request) {
                return response.map(Some);
            }
        }
        if let LocalDaemonRequest::CompletePrompt(request) = request {
            let response = self
                .agent_runtime
                .dispatch_prompt_complete(command, request.clone())
                .await?;
            return self
                .redact_result_for_user(Ok(response), caller_user_id)
                .map(Some);
        }
        if is_workflow_command(request) {
            let response = self
                .workflow_runtime
                .with_command_authority(&self.runtime_state)
                .dispatch_workflow_command(command.clone(), request.clone())
                .await?;
            return self
                .redact_workflow_result_for_user(Ok(response), caller_user_id, request)
                .map(Some);
        }
        if let LocalDaemonRequest::GetProviderRun(request) = request {
            if let Some(response) = projected_provider_run_response(
                &self.provider_run_projection,
                request,
                caller_user_id,
            )? {
                return Ok(Some(response));
            }
        }
        if matches!(request, LocalDaemonRequest::GetKernelResourceTelemetry(_)) {
            return execute_kernel_resource_telemetry_request(self.config_projection.snapshot())
                .map(Some);
        }
        if matches!(request, LocalDaemonRequest::GetDaemonHealth(_)) {
            return execute_daemon_health_request(
                self.daemon_health_projection_input(0),
                request.clone(),
            )
            .await
            .map(Some);
        }
        Ok(None)
    }
}
