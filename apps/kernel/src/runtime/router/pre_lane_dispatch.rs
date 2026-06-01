use std::sync::Arc;

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::agent_utility_executor::execute_agent_utility_request;
use crate::runtime::capability_registry::execute_capability_registry_request;
use crate::runtime::command::KernelCommand;
use crate::runtime::daemon_health_projection::execute_daemon_health_request;
use crate::runtime::provider_catalog_control::execute_provider_catalog_request;
use crate::runtime::provider_process_control::projected_provider_processes_response;
use crate::runtime::provider_run_control::projected_provider_run_response;
use crate::runtime::relay_config_control::execute_relay_config_request;
use crate::runtime::remote_relay_inventory::execute_remote_relay_inventory_request;
use crate::runtime::session_read_control::{
    projected_session_inspection_response, projected_session_read_response,
};
use crate::runtime::workflow_actor::is_workflow_command;
use crate::runtime::workspace_command_executor::execute_workspace_command_request;

use super::CommandRouter;

impl CommandRouter {
    pub(super) async fn dispatch_pre_lane(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
        caller_user_id: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        if let Some(response) = projected_session_read_response(
            &self.runtime_state,
            &self.session_projection,
            &self.provider_run_projection,
            &self.provider_launch_pending,
            &self.prompt_activity,
            &self.active_turns,
            request,
            caller_user_id,
        )
        .await
        {
            return response.map(Some);
        }
        match request {
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
            | LocalDaemonRequest::ListRemoteMachineKernels(_)) => {
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
            request @ (LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetProviderCommandCatalogs(_)) => {
                return execute_provider_catalog_request(
                    &self.provider_catalog_projection,
                    &self.config_projection,
                    request.clone(),
                )
                .await
                .map(Some);
            }
            request @ (LocalDaemonRequest::InstallMcpServer(_)
            | LocalDaemonRequest::UpdateMcpServer(_)
            | LocalDaemonRequest::UninstallMcpServer(_)
            | LocalDaemonRequest::ImportMcpServers(_)
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
                let vault_service = self
                    .config_projection
                    .snapshot()
                    .user_config
                    .credential_vault
                    .service;
                return execute_capability_registry_request(
                    request.clone(),
                    Some(vault_service.as_str()),
                )
                .map(Some);
            }
            _ => {}
        }
        if let Some(response) =
            projected_session_inspection_response(&self.session_projection, request, caller_user_id)
        {
            return response.map(Some);
        }
        if let LocalDaemonRequest::PumpTerminalOutput(request) = request {
            if let Some(response) = self.terminal_output_executor.projected_response(request) {
                return response.map(Some);
            }
        }
        if let LocalDaemonRequest::CompletePrompt(request) = request {
            return self
                .agent_runtime
                .dispatch_prompt_complete(command, request.clone())
                .await
                .map(Some);
        }
        if is_workflow_command(request) {
            return self
                .workflow_runtime
                .dispatch_workflow_command(command.clone(), request.clone())
                .await
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
        if let LocalDaemonRequest::ListProviderProcesses(request) = request {
            if let Some(response) = projected_provider_processes_response(
                &self.provider_process_projection,
                &self.provider_run_projection,
                request,
                caller_user_id,
            ) {
                return Ok(Some(response));
            }
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
