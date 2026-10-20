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
        super::retired_cloud_controls::require_kernel_authority(request)?;
        if let LocalDaemonRequest::AttachSessionView(request) = request {
            return self.attach_session_view(command, request.clone()).await.map(Some);
        }
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
        if let LocalDaemonRequest::PreflightProviderAccountPortability(request) = request {
            return crate::runtime::provider_account_portability::execute_preflight(
                &self.config_projection.snapshot(),
                &self.provider_account_profiles,
                command,
                request,
            )
            .map(Some);
        }
        if matches!(
            request,
            LocalDaemonRequest::AddSshMachine(_) | LocalDaemonRequest::RemoveSshMachine(_)
        ) {
            return crate::runtime::ssh_machine_control::execute(
                self.config_projection.snapshot(),
                command,
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
            LocalDaemonRequest::GetTerminalCommandCatalog(_) => {
                return terminal_command_catalog_response().map(Some);
            }
            LocalDaemonRequest::ListProviderProcesses(request) => {
                if let Some(processes) = self
                    .provider_process_projection
                    .list(request.provider.as_deref())
                {
                    let processes = provider_processes_visible_to_user_from_projection(
                        processes,
                        &self.provider_run_projection,
                        caller_user_id,
                    );
                    return Ok(Some(LocalDaemonResponse::ProviderProcessesListed {
                        processes,
                    }));
                }
            }
            request @ LocalDaemonRequest::RelayStatus(_) => {
                return execute_relay_config_request(
                    &self.runtime_state,
                    Arc::clone(&self.relay_state),
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request.clone(),
                )
                .await
                .map(Some);
            }
            request @ (LocalDaemonRequest::ListRemoteMachines(_)
            | LocalDaemonRequest::ListRemoteMachineKernels(_)
            | LocalDaemonRequest::QueryFreshRemoteMachineKernels(_)) => {
                return execute_remote_relay_inventory_request(
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                    self.remote_relay_inventory_projection.clone(),
                    request.clone(),
                )
                .await
                .map(Some);
            }
            request @ (LocalDaemonRequest::SearchWorkspaceDirectories(_)
            | LocalDaemonRequest::CreateWorkspaceDirectory(_)
            | LocalDaemonRequest::ListWorkspaceWorktrees(_)
            | LocalDaemonRequest::CreateWorkspaceWorktree(_)
            | LocalDaemonRequest::DeleteWorkspaceWorktree(_)
            | LocalDaemonRequest::CreateWorkspacePullRequest(_)
            | LocalDaemonRequest::GetWorkspaceGitOverview(_)
            | LocalDaemonRequest::ListWorkspaceFiles(_)
            | LocalDaemonRequest::GetWorkspaceFileContent(_)
            | LocalDaemonRequest::CommitWorkspaceChanges(_)
            | LocalDaemonRequest::PushWorkspaceBranch(_)
            | LocalDaemonRequest::CommitAndPushWorkspaceChanges(_)) => {
                return execute_workspace_command_request(
                    &self.runtime_state,
                    &self.session_projection,
                    request.clone(),
                )
                .await
                .map(Some);
            }
            request @ (LocalDaemonRequest::RunAgentUtility(_)
            | LocalDaemonRequest::GenerateWorkspaceCommitMessage(_)) => {
                return execute_agent_utility_request(
                    &self.runtime_state,
                    &self.config_projection,
                    request.clone(),
                )
                .await
                .map(Some);
            }
            LocalDaemonRequest::AdjustProjectEnvironment(request) => {
                return self
                    .runtime_state
                    .start_project_environment_adjustment(request.clone(), caller_user_id)
                    .await
                    .map(Some);
            }
            request if request.unsupported_environment_capability().is_some() => {
                return Ok(Some(LocalDaemonResponse::EnvironmentUnsupportedFeature {
                    feature: request
                        .unsupported_environment_capability()
                        .expect("matched unsupported feature"),
                    supported_schema: 1,
                }));
            }
            LocalDaemonRequest::GetProjectEnvironment(request) => {
                return self
                    .runtime_state
                    .get_project_environment(request.clone(), caller_user_id)
                    .map(Some);
            }
            LocalDaemonRequest::GetProjectEnvironmentManifest(request) => {
                return self
                    .runtime_state
                    .get_project_environment_manifest(request.clone(), caller_user_id)
                    .await
                    .map(Some);
            }
            request @ (LocalDaemonRequest::StartProjectEnvironmentSetup(_)
            | LocalDaemonRequest::GetProjectEnvironmentSetupStatus(_)
            | LocalDaemonRequest::CancelProjectEnvironmentSetup(_)
            | LocalDaemonRequest::RetryProjectEnvironmentSetup(_)) => {
                return self
                    .runtime_state
                    .execute_project_environment_setup_request(request.clone(), caller_user_id)
                    .await
                    .map(Some);
            }
            request @ (LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetClientCatalogSnapshot(_)
            | LocalDaemonRequest::GetClientCatalogRevisions(_)
            | LocalDaemonRequest::GetProviderCommandCatalogs(_)) => {
                return execute_provider_catalog_request(
                    &self.provider_catalog_projection,
                    &self.config_projection,
                    self.runtime_state.provider_account_profile_registry(),
                    caller_user_id,
                    request.clone(),
                )
                .await
                .map(Some);
            }
            request @ (LocalDaemonRequest::GetEventGeneratorCatalogLanding(_)
            | LocalDaemonRequest::SearchEventGeneratorCatalog(_)
            | LocalDaemonRequest::BrowseEventGeneratorCategory(_)
            | LocalDaemonRequest::GetEventGeneratorDetail(_)
            | LocalDaemonRequest::BrowseEventGeneratorEvents(_)
            | LocalDaemonRequest::StartEventGeneratorAuthorization(_)
            | LocalDaemonRequest::ListEventGeneratorResources(_)
            | LocalDaemonRequest::ListEventConnections(_)
            | LocalDaemonRequest::GetEventConnection(_)
            | LocalDaemonRequest::InstallEventConnection(_)
            | LocalDaemonRequest::ObserveEventConnectionAuthorization(_)
            | LocalDaemonRequest::RefreshEventConnection(_)
            | LocalDaemonRequest::TestEventConnection(_)
            | LocalDaemonRequest::ReconnectEventConnection(_)
            | LocalDaemonRequest::ListEventConnectionResources(_)
            | LocalDaemonRequest::ListEventConnectionDependencies(_)
            | LocalDaemonRequest::GetEventDeliveryStatus(_)) => {
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
            request @ (LocalDaemonRequest::InstallMcpServer(_)
            | LocalDaemonRequest::UpdateMcpServer(_)
            | LocalDaemonRequest::UninstallMcpServer(_)
            | LocalDaemonRequest::ImportMcpServers(_)
            | LocalDaemonRequest::ImportProviderCapabilities(_)
            | LocalDaemonRequest::GetMcpServer(_)
            | LocalDaemonRequest::ListMcpServers(_)
            | LocalDaemonRequest::RegisterEnvironment(_)
            | LocalDaemonRequest::RemoveEnvironment(_)
            | LocalDaemonRequest::GetEnvironment(_)
            | LocalDaemonRequest::ListEnvironments(_)
            | LocalDaemonRequest::ValidateScript(_)
            | LocalDaemonRequest::RegisterScript(_)
            | LocalDaemonRequest::RemoveScript(_)
            | LocalDaemonRequest::GetScript(_)
            | LocalDaemonRequest::ListScripts(_)
            | LocalDaemonRequest::RegisterCredential(_)
            | LocalDaemonRequest::UpsertCredential(_)
            | LocalDaemonRequest::RemoveCredential(_)
            | LocalDaemonRequest::GetCredential(_)
            | LocalDaemonRequest::ListCredentials(_)
            | LocalDaemonRequest::RegisterConnector(_)
            | LocalDaemonRequest::UpsertConnector(_)
            | LocalDaemonRequest::RegisterConnectorAdapter(_)
            | LocalDaemonRequest::RemoveConnectorAdapter(_)
            | LocalDaemonRequest::GetConnectorAdapter(_)
            | LocalDaemonRequest::ListConnectorAdapters(_)
            | LocalDaemonRequest::RemoveConnector(_)
            | LocalDaemonRequest::GetConnector(_)
            | LocalDaemonRequest::ListConnectors(_)
            | LocalDaemonRequest::TestConnector(_)
            | LocalDaemonRequest::UpsertSkill(_)
            | LocalDaemonRequest::InstallSkill(_)
            | LocalDaemonRequest::UpdateSkill(_)
            | LocalDaemonRequest::UninstallSkill(_)
            | LocalDaemonRequest::ImportSkills(_)
            | LocalDaemonRequest::GetSkill(_)
            | LocalDaemonRequest::ListSkills(_)) => {
                let credential_vault = self
                    .config_projection
                    .snapshot()
                    .user_config
                    .credential_vault;
                return execute_capability_registry_request(
                    &self.runtime_state,
                    request.clone(),
                    credential_vault,
                )
                .await
                .map(Some);
            }
            _ => {}
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
