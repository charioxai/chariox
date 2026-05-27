use std::sync::Arc;

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::cloud_relay_executor::execute_cloud_relay_request;
use crate::runtime::command::{KernelCommand, KernelCommandPriority};
use crate::runtime::history_executor::execute_history_request;
use crate::runtime::kernel_lifecycle_executor::execute_kernel_lifecycle_request;
use crate::runtime::pairing_invite_executor::execute_pairing_request;
use crate::runtime::provider_process_control::execute_provider_process_request;
use crate::runtime::relay_config_control::execute_relay_config_request;
use crate::runtime::remote_machine_registry::execute_remote_machine_registry_request;
use crate::runtime::session_collaboration_executor::execute_session_collaboration_request;
use crate::runtime::slice_command_executor::execute_slice_request;
use crate::runtime::user_config_executor::execute_user_config_request;

use super::CommandRouter;

impl CommandRouter {
    pub(super) async fn dispatch_refresh_tracked(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            request @ LocalDaemonRequest::ConfigureRelay(_) => {
                execute_relay_config_request(
                    &self.runtime_state,
                    Arc::clone(&self.relay_state),
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::CloudRelayStatus(_)
            | LocalDaemonRequest::StartCloudRelayLogin(_)
            | LocalDaemonRequest::PollCloudRelayLogin(_)
            | LocalDaemonRequest::LogoutCloudRelay(_)
            | LocalDaemonRequest::PairCloudRelayClient(_)
            | LocalDaemonRequest::PairCloudRelayMachine(_)
            | LocalDaemonRequest::ConnectCloudRelay(_)
            | LocalDaemonRequest::IssueCloudRelayClientToken(_)
            | LocalDaemonRequest::CreateCloudSessionInvite(_)
            | LocalDaemonRequest::ShowCloudSessionInvite(_)
            | LocalDaemonRequest::AcceptCloudSessionInvite(_)
            | LocalDaemonRequest::RevokeCloudSessionInvite(_)
            | LocalDaemonRequest::ListCloudSessionMembers(_)
            | LocalDaemonRequest::ListCloudCollaborators(_)) => {
                execute_cloud_relay_request(
                    &self.runtime_state,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    Arc::clone(&self.relay_state),
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::GetUserConfig(_)
            | LocalDaemonRequest::GetUserConfigSchema(_)
            | LocalDaemonRequest::SetUserConfigValue(_)
            | LocalDaemonRequest::UnsetUserConfigValue(_)
            | LocalDaemonRequest::SetCredentialSecret(_)
            | LocalDaemonRequest::DeleteCredentialSecret(_)) => {
                execute_user_config_request(&self.config_projection, &self.runtime_state, request)
                    .await
            }
            request @ (LocalDaemonRequest::ListSlices(_)
            | LocalDaemonRequest::CreateSlice(_)
            | LocalDaemonRequest::GetSlice(_)
            | LocalDaemonRequest::StartSlice(_)
            | LocalDaemonRequest::StopSlice(_)
            | LocalDaemonRequest::DeleteSlice(_)
            | LocalDaemonRequest::ImportSliceProviderAuth(_)
            | LocalDaemonRequest::GetSliceDisplayEndpoint(_)) => {
                execute_slice_request(&self.runtime_state, &self.config_projection, request).await
            }
            request @ LocalDaemonRequest::DeleteKernel(_) => {
                execute_kernel_lifecycle_request(
                    &self.config_projection,
                    &self.runtime_state,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::ApproveRemoteMachine(_)
            | LocalDaemonRequest::ForgetRemoteMachine(_)
            | LocalDaemonRequest::RenameRemoteMachine(_)) => {
                execute_remote_machine_registry_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::ListSessionMembers(_)
            | LocalDaemonRequest::CreateSessionInvite(_)
            | LocalDaemonRequest::JoinSessionInvite(_)
            | LocalDaemonRequest::RevokeSessionInvite(_)
            | LocalDaemonRequest::CreateWorkspaceLink(_)
            | LocalDaemonRequest::ListWorkspaceLinks(_)
            | LocalDaemonRequest::ShowWorkspaceLink(_)
            | LocalDaemonRequest::AttachWorkspaceLink(_)
            | LocalDaemonRequest::DetachWorkspaceLink(_)
            | LocalDaemonRequest::GetWorkspaceLiveSyncStatus(_)) => {
                execute_session_collaboration_request(
                    &self.runtime_state,
                    &self.config_projection,
                    &command,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::CreatePairingInvite(_)
            | LocalDaemonRequest::JoinPairingInvite(_)
            | LocalDaemonRequest::CreateTerminalPairingLink(_)
            | LocalDaemonRequest::JoinTerminalPairingLink(_)
            | LocalDaemonRequest::ListTerminals(_)
            | LocalDaemonRequest::ListPairedClients(_)
            | LocalDaemonRequest::RecordPairedClient(_)
            | LocalDaemonRequest::RevokePairedClient(_)) => {
                execute_pairing_request(
                    &self.runtime_state,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::GetSessionHistory(_)
            | LocalDaemonRequest::GetPromptInputHistory(_)
            | LocalDaemonRequest::RecordPromptInputHistory(_)
            | LocalDaemonRequest::QueryRecall(_)
            | LocalDaemonRequest::SearchRecall(_)
            | LocalDaemonRequest::SemanticSearchRecall(_)) => {
                execute_history_request(
                    self.history_store.clone(),
                    self.operational_history_store.clone(),
                    self.history_projection.clone(),
                    &self.runtime_state,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::PumpTerminalOutput(request) => {
                self.terminal_output_executor.execute(request).await
            }
            request @ LocalDaemonRequest::TeardownProviderProcesses(_) => {
                execute_provider_process_request(
                    &self.runtime_state,
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    &self.provider_process_projection,
                    request,
                )
                .await
            }
            request => match command.priority {
                KernelCommandPriority::Interactive => {
                    self.dispatch_interactive(command, request).await
                }
                KernelCommandPriority::Normal | KernelCommandPriority::Background => {
                    self.dispatch_normal_or_background(command, request).await
                }
            },
        }
    }
}
