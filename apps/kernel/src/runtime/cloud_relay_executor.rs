use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::cloud_relay_connection_executor::{
    execute_cloud_relay_status_request, execute_connect_cloud_relay_request,
    execute_issue_cloud_relay_client_token_request,
};
use crate::runtime::cloud_relay_login_executor::{
    execute_logout_cloud_relay_request, execute_poll_cloud_relay_login_request,
    execute_start_cloud_relay_login_request,
};
use crate::runtime::cloud_relay_pairing_executor::{
    execute_pair_cloud_relay_client_request, execute_pair_cloud_relay_machine_request,
};
use crate::runtime::cloud_session_control_executor::{
    execute_accept_cloud_session_invite_request, execute_create_cloud_session_invite_request,
    execute_list_cloud_collaborators_request, execute_list_cloud_session_members_request,
    execute_revoke_cloud_session_invite_request, execute_show_cloud_session_invite_request,
};
use crate::runtime::projection::{DaemonConfigProjectionStore, ProviderCatalogProjectionStore};
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_client::RelayClientState;

pub(crate) async fn execute_cloud_relay_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    provider_catalog_projection: &ProviderCatalogProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::CloudRelayStatus(_) => {
            execute_cloud_relay_status_request(config_projection).await
        }
        LocalDaemonRequest::StartCloudRelayLogin(request) => {
            execute_start_cloud_relay_login_request(request).await
        }
        LocalDaemonRequest::PollCloudRelayLogin(request) => {
            execute_poll_cloud_relay_login_request(runtime_state, request).await
        }
        LocalDaemonRequest::LogoutCloudRelay(request) => {
            execute_logout_cloud_relay_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::PairCloudRelayClient(request) => {
            execute_pair_cloud_relay_client_request(runtime_state, config_projection, request).await
        }
        LocalDaemonRequest::PairCloudRelayMachine(request) => {
            execute_pair_cloud_relay_machine_request(
                runtime_state,
                config_projection,
                provider_catalog_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::ConnectCloudRelay(request) => {
            execute_connect_cloud_relay_request(
                runtime_state,
                config_projection,
                provider_catalog_projection,
                relay_state,
                request,
            )
            .await
        }
        LocalDaemonRequest::IssueCloudRelayClientToken(request) => {
            execute_issue_cloud_relay_client_token_request(
                runtime_state,
                config_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::CreateCloudSessionInvite(request) => {
            execute_create_cloud_session_invite_request(runtime_state, config_projection, request)
                .await
        }
        LocalDaemonRequest::ShowCloudSessionInvite(request) => {
            execute_show_cloud_session_invite_request(config_projection, request).await
        }
        LocalDaemonRequest::AcceptCloudSessionInvite(request) => {
            execute_accept_cloud_session_invite_request(runtime_state, config_projection, request)
                .await
        }
        LocalDaemonRequest::RevokeCloudSessionInvite(request) => {
            execute_revoke_cloud_session_invite_request(runtime_state, config_projection, request)
                .await
        }
        LocalDaemonRequest::ListCloudSessionMembers(request) => {
            execute_list_cloud_session_members_request(runtime_state, config_projection, request)
                .await
        }
        LocalDaemonRequest::ListCloudCollaborators(request) => {
            execute_list_cloud_collaborators_request(runtime_state, config_projection, request)
                .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "unsupported cloud relay request".to_string(),
        }),
    }
}
