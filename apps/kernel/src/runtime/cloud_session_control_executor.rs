use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    AcceptCloudSessionInviteRequest, CreateCloudSessionInviteRequest,
    ListCloudCollaboratorsRequest, ListCloudSessionMembersRequest, LocalDaemonResponse,
    RevokeCloudSessionInviteRequest, ShowCloudSessionInviteRequest,
};
use crate::runtime::cloud_api_client::{
    accept_cloud_session_invite, create_cloud_session_invite, list_cloud_collaborators,
    list_cloud_session_members, revoke_cloud_session_invite, show_cloud_session_invite,
};
use crate::runtime::cloud_relay_profile_store::{
    clear_cloud_profile_if_stale, required_cloud_relay_profile,
    required_cloud_relay_profile_with_session,
};
use crate::runtime::projection::DaemonConfigProjectionStore;

pub(crate) async fn execute_create_cloud_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: CreateCloudSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let invite = match create_cloud_session_invite(&profile, request).await {
        Ok(invite) => invite,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudSessionInviteCreated { invite })
}

pub(crate) async fn execute_show_cloud_session_invite_request(
    config_projection: &DaemonConfigProjectionStore,
    request: ShowCloudSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile(config_projection)?;
    let invite = show_cloud_session_invite(&profile, request).await?;
    Ok(LocalDaemonResponse::CloudSessionInviteShown { invite })
}

pub(crate) async fn execute_accept_cloud_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: AcceptCloudSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let acceptance = match accept_cloud_session_invite(&profile, request).await {
        Ok(acceptance) => acceptance,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudSessionInviteAccepted { acceptance })
}

pub(crate) async fn execute_revoke_cloud_session_invite_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: RevokeCloudSessionInviteRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let revoked = match revoke_cloud_session_invite(&profile, request).await {
        Ok(revoked) => revoked,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudSessionInviteRevoked {
        invite_id: revoked.invite_id,
        status: revoked.status,
    })
}

pub(crate) async fn execute_list_cloud_session_members_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    request: ListCloudSessionMembersRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let listed = match list_cloud_session_members(&profile, request).await {
        Ok(listed) => listed,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudSessionMembersListed {
        session_id: listed.session_id,
        members: listed.members,
    })
}

pub(crate) async fn execute_list_cloud_collaborators_request(
    app: &Arc<Mutex<DaemonApp>>,
    config_projection: &DaemonConfigProjectionStore,
    _request: ListCloudCollaboratorsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = required_cloud_relay_profile_with_session(config_projection)?;
    let collaborators = match list_cloud_collaborators(&profile).await {
        Ok(collaborators) => collaborators,
        Err(error) => {
            clear_cloud_profile_if_stale(app, config_projection, &error).await?;
            return Err(error);
        }
    };
    Ok(LocalDaemonResponse::CloudCollaboratorsListed { collaborators })
}
