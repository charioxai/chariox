//! Cloud session invite/member/collaborator API client.

use serde::Deserialize;

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::{
    AcceptCloudSessionInviteRequest, CloudCollaborator, CloudSessionInvite,
    CloudSessionInviteAcceptance, CloudSessionInviteDetails, CloudSessionMember,
    CreateCloudSessionInviteRequest, ListCloudSessionMembersRequest,
    RevokeCloudSessionInviteRequest, ShowCloudSessionInviteRequest,
};

use super::{cloud_url_component, get_cloud_json, post_cloud_json, post_cloud_json_dynamic};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionInviteResponse {
    invite_id: String,
    invite_token: String,
    session_id: String,
    account_id: String,
    created_by_user_id: String,
    expires_at: Option<String>,
    max_uses: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionInviteDetailsResponse {
    invite_id: String,
    session_id: String,
    account_id: String,
    created_by_user_id: String,
    display_name: Option<String>,
    expires_at: Option<String>,
    max_uses: Option<u32>,
    used_count: u32,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionInviteAcceptanceResponse {
    session_id: String,
    account_id: String,
    user_id: String,
    invited_by_user_id: String,
    joined_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionInviteRevokedResponse {
    invite_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionMembersResponse {
    session_id: String,
    members: Vec<CloudSessionMemberResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionMemberResponse {
    user_id: String,
    email: String,
    display_name: Option<String>,
    invited_by_user_id: Option<String>,
    joined_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudCollaboratorsResponse {
    collaborators: Vec<CloudCollaboratorResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudCollaboratorResponse {
    user_id: String,
    email: String,
    display_name: Option<String>,
    last_collaborated_at: String,
    shared_session_count: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct CloudSessionInviteRevocation {
    pub(crate) invite_id: String,
    pub(crate) status: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CloudSessionMembers {
    pub(crate) session_id: String,
    pub(crate) members: Vec<CloudSessionMember>,
}

pub(crate) async fn create_cloud_session_invite(
    profile: &PersistedCloudRelayProfile,
    request: CreateCloudSessionInviteRequest,
) -> Result<CloudSessionInvite, DaemonError> {
    let invite: CloudSessionInviteResponse = post_cloud_json(
        profile.api_url.clone(),
        "/sessions/invites",
        serde_json::json!({
            "sessionToken": profile.cloud_session_token,
            "accountId": profile.account_id,
            "sessionId": request.session_id,
            "displayName": request.display_name,
            "expiresInMs": request.expires_in_ms,
            "maxUses": request.max_uses,
            "collaborationLevel": request.collaboration_level,
        }),
    )
    .await?;
    Ok(cloud_session_invite_from_response(invite))
}

pub(crate) async fn show_cloud_session_invite(
    profile: &PersistedCloudRelayProfile,
    request: ShowCloudSessionInviteRequest,
) -> Result<CloudSessionInviteDetails, DaemonError> {
    let invite: CloudSessionInviteDetailsResponse = get_cloud_json(
        profile.api_url.clone(),
        format!(
            "/sessions/invites/{}",
            cloud_url_component(&request.invite_token)
        ),
    )
    .await?;
    Ok(cloud_session_invite_details_from_response(invite))
}

pub(crate) async fn accept_cloud_session_invite(
    profile: &PersistedCloudRelayProfile,
    request: AcceptCloudSessionInviteRequest,
) -> Result<CloudSessionInviteAcceptance, DaemonError> {
    let acceptance: CloudSessionInviteAcceptanceResponse = post_cloud_json_dynamic(
        profile.api_url.clone(),
        format!(
            "/sessions/invites/{}/accept",
            cloud_url_component(&request.invite_token)
        ),
        serde_json::json!({
            "sessionToken": profile.cloud_session_token,
        }),
    )
    .await?;
    Ok(cloud_session_invite_acceptance_from_response(acceptance))
}

pub(crate) async fn revoke_cloud_session_invite(
    profile: &PersistedCloudRelayProfile,
    request: RevokeCloudSessionInviteRequest,
) -> Result<CloudSessionInviteRevocation, DaemonError> {
    let revoked: CloudSessionInviteRevokedResponse = post_cloud_json(
        profile.api_url.clone(),
        "/sessions/invites/revoke",
        serde_json::json!({
            "sessionToken": profile.cloud_session_token,
            "accountId": profile.account_id,
            "sessionId": request.session_id,
            "inviteId": request.invite_id,
        }),
    )
    .await?;
    Ok(CloudSessionInviteRevocation {
        invite_id: revoked.invite_id,
        status: revoked.status,
    })
}

pub(crate) async fn list_cloud_session_members(
    profile: &PersistedCloudRelayProfile,
    request: ListCloudSessionMembersRequest,
) -> Result<CloudSessionMembers, DaemonError> {
    let listed: CloudSessionMembersResponse = get_cloud_json(
        profile.api_url.clone(),
        cloud_session_members_path(profile, &request.session_id),
    )
    .await?;
    Ok(CloudSessionMembers {
        session_id: listed.session_id,
        members: listed
            .members
            .into_iter()
            .map(cloud_session_member_from_response)
            .collect(),
    })
}

pub(crate) async fn list_cloud_collaborators(
    profile: &PersistedCloudRelayProfile,
) -> Result<Vec<CloudCollaborator>, DaemonError> {
    let listed: CloudCollaboratorsResponse =
        get_cloud_json(profile.api_url.clone(), cloud_collaborators_path(profile)).await?;
    Ok(listed
        .collaborators
        .into_iter()
        .map(cloud_collaborator_from_response)
        .collect())
}

fn cloud_session_invite_from_response(response: CloudSessionInviteResponse) -> CloudSessionInvite {
    CloudSessionInvite {
        invite_id: response.invite_id,
        invite_token: response.invite_token,
        session_id: response.session_id,
        account_id: response.account_id,
        created_by_user_id: response.created_by_user_id,
        expires_at: response.expires_at,
        max_uses: response.max_uses,
    }
}

fn cloud_session_invite_details_from_response(
    response: CloudSessionInviteDetailsResponse,
) -> CloudSessionInviteDetails {
    CloudSessionInviteDetails {
        invite_id: response.invite_id,
        session_id: response.session_id,
        account_id: response.account_id,
        created_by_user_id: response.created_by_user_id,
        display_name: response.display_name,
        expires_at: response.expires_at,
        max_uses: response.max_uses,
        used_count: response.used_count,
        status: response.status,
    }
}

fn cloud_session_invite_acceptance_from_response(
    response: CloudSessionInviteAcceptanceResponse,
) -> CloudSessionInviteAcceptance {
    CloudSessionInviteAcceptance {
        session_id: response.session_id,
        account_id: response.account_id,
        user_id: response.user_id,
        invited_by_user_id: response.invited_by_user_id,
        joined_at: response.joined_at,
    }
}

fn cloud_session_member_from_response(response: CloudSessionMemberResponse) -> CloudSessionMember {
    CloudSessionMember {
        user_id: response.user_id,
        email: response.email,
        display_name: response.display_name,
        invited_by_user_id: response.invited_by_user_id,
        joined_at: response.joined_at,
    }
}

fn cloud_collaborator_from_response(response: CloudCollaboratorResponse) -> CloudCollaborator {
    CloudCollaborator {
        user_id: response.user_id,
        email: response.email,
        display_name: response.display_name,
        last_collaborated_at: response.last_collaborated_at,
        shared_session_count: response.shared_session_count,
    }
}

fn cloud_session_members_path(profile: &PersistedCloudRelayProfile, session_id: &str) -> String {
    format!(
        "/sessions/members?sessionToken={}&accountId={}&sessionId={}",
        cloud_url_component(profile.cloud_session_token.as_deref().unwrap_or_default()),
        cloud_url_component(&profile.account_id),
        cloud_url_component(session_id),
    )
}

fn cloud_collaborators_path(profile: &PersistedCloudRelayProfile) -> String {
    format!(
        "/collaborators/recent?sessionToken={}&accountId={}",
        cloud_url_component(profile.cloud_session_token.as_deref().unwrap_or_default()),
        cloud_url_component(&profile.account_id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud_profile() -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            api_url: "https://cloud.test".to_string(),
            email: "user@example.test".to_string(),
            account_id: "account/1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "acct".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: Some("client-1".to_string()),
            client_alias: None,
            machine_id: Some("machine-1".to_string()),
            machine_alias: None,
            machine_credential: None,
            cloud_session_token: Some("session token".to_string()),
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        }
    }

    #[test]
    fn cloud_session_collaboration_paths_encode_query_values() {
        let profile = cloud_profile();

        assert_eq!(
            cloud_session_members_path(&profile, "session/1"),
            "/sessions/members?sessionToken=session%20token&accountId=account%2F1&sessionId=session%2F1"
        );
        assert_eq!(
            cloud_collaborators_path(&profile),
            "/collaborators/recent?sessionToken=session%20token&accountId=account%2F1"
        );
    }
}
