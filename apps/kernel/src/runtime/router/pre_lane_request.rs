//! MP-08 / MP-10 / MP-11: select a single pre-lane handler before polling it.
use std::{future::Future, pin::Pin, sync::Arc};

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::agent_utility_executor::execute_agent_utility_request;
use crate::runtime::capability_registry::execute_capability_registry_request;
use crate::runtime::event_catalog_control::execute_event_catalog_request_with_client;
use crate::runtime::managed_bootstrap_observation_control::execute_managed_bootstrap_observation_request;
use crate::runtime::managed_context_outbound_control::{
    execute_managed_context_outbound_request, OutboundRequester,
};
use crate::runtime::managed_context_target_control::execute_managed_context_target_request;
use crate::runtime::provider_catalog_control::execute_provider_catalog_request;
use crate::runtime::provider_process_control::provider_processes_visible_to_user_from_projection;
use crate::runtime::relay_config_control::execute_relay_config_request;
use crate::runtime::remote_relay_inventory::execute_remote_relay_inventory_request;
use crate::runtime::terminal_command_catalog::terminal_command_catalog_response;
use crate::runtime::workspace_command_executor::execute_workspace_command_request;

use super::CommandRouter;

type PreLaneRequestFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<LocalDaemonResponse>, DaemonError>> + Send + 'a>>;

impl CommandRouter {
    // Keep unrelated request handlers out of the active router poll frame.
    #[inline(never)]
    pub(super) fn select_pre_lane_request<'a>(
        &'a self,
        request: &'a LocalDaemonRequest,
        caller_user_id: &'a str,
        session_id: Option<&'a str>,
    ) -> Option<PreLaneRequestFuture<'a>> {
        match request {
            LocalDaemonRequest::ObserveKernelRuntimeIdentity(request) => {
                Some(Box::pin(async move {
                    execute_managed_bootstrap_observation_request(
                        self.config_projection.snapshot(),
                        self.managed_kernel_registration.clone(),
                        caller_user_id,
                        request.clone(),
                    )
                    .await
                    .map(Some)
                }))
            }
            request @ (LocalDaemonRequest::StartManagedContextTransfer(_)
            | LocalDaemonRequest::GetManagedContextTransferStatus(_)) => {
                Some(Box::pin(async move {
                    execute_managed_context_outbound_request(
                        self.config_projection.snapshot(),
                        Arc::clone(&self.relay_state),
                        self.managed_context_outbound.clone(),
                        self.provider_account_profiles.clone(),
                        self.runtime_state.clone(),
                        OutboundRequester {
                            user_id: caller_user_id,
                            session_id,
                        },
                        request.clone(),
                    )
                    .map(Some)
                }))
            }
            request @ LocalDaemonRequest::GetManagedContextLaunchTarget(_) => {
                Some(Box::pin(async move {
                    let response = execute_managed_context_target_request(
                        self.config_projection.snapshot(),
                        self.managed_kernel_registration.clone(),
                        self.managed_context_transfers.clone(),
                        caller_user_id,
                        request.clone(),
                    )?;
                    if let LocalDaemonResponse::ManagedContextLaunchTarget { target } = &response {
                        self.runtime_state
                            .ensure_managed_context_project(target, caller_user_id)?;
                    }
                    Ok(Some(response))
                }))
            }
            LocalDaemonRequest::GetTerminalCommandCatalog(_) => Some(Box::pin(async move {
                terminal_command_catalog_response().map(Some)
            })),
            LocalDaemonRequest::ListProviderProcesses(request) => Some(Box::pin(async move {
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
                Ok(None)
            })),
            request @ LocalDaemonRequest::RelayStatus(_) => Some(Box::pin(async move {
                execute_relay_config_request(
                    &self.runtime_state,
                    Arc::clone(&self.relay_state),
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request.clone(),
                )
                .await
                .map(Some)
            })),
            request @ (LocalDaemonRequest::ListRemoteMachines(_)
            | LocalDaemonRequest::ListRemoteMachineKernels(_)
            | LocalDaemonRequest::QueryFreshRemoteMachineKernels(_)) => {
                Some(Box::pin(async move {
                    execute_remote_relay_inventory_request(
                        Arc::clone(&self.relay_state),
                        self.config_projection.clone(),
                        self.remote_relay_inventory_projection.clone(),
                        request.clone(),
                    )
                    .await
                    .map(Some)
                }))
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
            | LocalDaemonRequest::CommitAndPushWorkspaceChanges(_)) => Some(Box::pin(async move {
                execute_workspace_command_request(
                    &self.runtime_state,
                    &self.session_projection,
                    request.clone(),
                )
                .await
                .map(Some)
            })),
            request @ (LocalDaemonRequest::RunAgentUtility(_)
            | LocalDaemonRequest::GenerateWorkspaceCommitMessage(_)) => {
                Some(Box::pin(async move {
                    execute_agent_utility_request(
                        &self.runtime_state,
                        &self.config_projection,
                        caller_user_id,
                        request.clone(),
                    )
                    .await
                    .map(Some)
                }))
            }
            LocalDaemonRequest::GetProjectEnvironment(request) => Some(Box::pin(async move {
                self.runtime_state
                    .get_project_environment(request.clone(), caller_user_id)
                    .map(Some)
            })),
            LocalDaemonRequest::SaveProjectEnvironment(request) => Some(Box::pin(async move {
                self.runtime_state
                    .save_project_environment(request.clone(), caller_user_id)
                    .map(Some)
            })),
            LocalDaemonRequest::RunProjectEnvironmentSetup(request) => Some(Box::pin(async move {
                self.runtime_state
                    .run_project_environment_setup(request.clone(), caller_user_id)
                    .map(Some)
            })),
            LocalDaemonRequest::ImportProjectEnvironmentEntries(request) => {
                Some(Box::pin(async move {
                    self.runtime_state
                        .import_project_environment_entries(request.clone(), caller_user_id)
                        .map(Some)
                }))
            }
            LocalDaemonRequest::DraftProjectEnvironment(request) => Some(Box::pin(async move {
                self.runtime_state
                    .draft_project_environment(request.clone(), caller_user_id)
                    .map(Some)
            })),
            LocalDaemonRequest::RunRemoteProjectEnvironmentSetup(request) => {
                Some(Box::pin(async move {
                    self.runtime_state
                        .run_remote_project_environment_setup(request.clone(), caller_user_id)
                        .await
                        .map(Some)
                }))
            }
            LocalDaemonRequest::RemoteEnvironmentOperation(request) => Some(Box::pin(async move {
                self.runtime_state
                    .remote_environment_operation(request.clone(), caller_user_id)
                    .await
                    .map(Some)
            })),
            LocalDaemonRequest::CheckProjectEnvironment(request) => Some(Box::pin(async move {
                self.runtime_state
                    .check_project_environment(request.clone(), caller_user_id)
                    .map(Some)
            })),
            LocalDaemonRequest::ResolveEnvironmentProposal(request) => Some(Box::pin(async move {
                self.runtime_state
                    .resolve_environment_proposal(request.clone(), caller_user_id)
                    .map(Some)
            })),
            LocalDaemonRequest::GetEnvironmentOperation(request) => Some(Box::pin(async move {
                self.runtime_state
                    .get_environment_operation(request.clone(), caller_user_id)
                    .await
                    .map(Some)
            })),
            LocalDaemonRequest::CancelEnvironmentOperation(request) => Some(Box::pin(async move {
                self.runtime_state
                    .cancel_environment_operation(request.clone(), caller_user_id)
                    .map(Some)
            })),
            request @ (LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetClientCatalogSnapshot(_)
            | LocalDaemonRequest::GetClientCatalogRevisions(_)
            | LocalDaemonRequest::GetProviderCommandCatalogs(_)) => Some(Box::pin(async move {
                execute_provider_catalog_request(
                    &self.provider_catalog_projection,
                    &self.config_projection,
                    self.runtime_state.provider_account_profile_registry(),
                    caller_user_id,
                    request.clone(),
                )
                .await
                .map(Some)
            })),
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
            | LocalDaemonRequest::ListEventConnectionEvents(_)
            | LocalDaemonRequest::ReplayEventConnectionEvent(_)
            | LocalDaemonRequest::ReconnectEventConnection(_)
            | LocalDaemonRequest::ListEventConnectionResources(_)
            | LocalDaemonRequest::ListEventConnectionDependencies(_)
            | LocalDaemonRequest::GetEventDeliveryStatus(_)) => Some(Box::pin(async move {
                execute_event_catalog_request_with_client(
                    &self.runtime_state,
                    &self.config_projection,
                    &self.aegs_management_http_client,
                    caller_user_id,
                    request.clone(),
                )
                .await
                .map(Some)
            })),
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
            | LocalDaemonRequest::ListSkills(_)) => Some(Box::pin(async move {
                let credential_vault = self
                    .config_projection
                    .snapshot()
                    .user_config
                    .credential_vault;
                execute_capability_registry_request(
                    &self.runtime_state,
                    request.clone(),
                    credential_vault,
                )
                .await
                .map(Some)
            })),
            _ => None,
        }
    }
}
