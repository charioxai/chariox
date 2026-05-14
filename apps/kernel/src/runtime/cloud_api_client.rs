use serde::Deserialize;

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::{
    AcceptCloudSessionInviteRequest, CloudCollaborator, CloudRelayProfile, CloudSessionInvite,
    CloudSessionInviteAcceptance, CloudSessionInviteDetails, CloudSessionMember,
    CreateCloudSessionInviteRequest, ListCloudSessionMembersRequest,
    RevokeCloudSessionInviteRequest, ShowCloudSessionInviteRequest,
};
use crate::runtime::cloud_relay_control::CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudDeviceStartResponse {
    pub(crate) device_code: String,
    pub(crate) user_code: String,
    pub(crate) verification_url: String,
    pub(crate) expires_at: String,
    pub(crate) interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudDevicePollResponse {
    pub(crate) status: String,
    pub(crate) interval_seconds: Option<u64>,
    pub(crate) expires_at: Option<String>,
    pub(crate) profile: Option<CloudDeviceProfileResponse>,
    pub(crate) cloud_session_token: Option<String>,
    pub(crate) cloud_session_expires_at: Option<String>,
    pub(crate) machine_credential: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudDeviceProfileResponse {
    pub(crate) email: String,
    pub(crate) account_id: String,
    pub(crate) user_id: String,
    pub(crate) account_slug: String,
    pub(crate) realm_id: String,
    pub(crate) relay_url: String,
    pub(crate) issuer_id: String,
    pub(crate) client_id: Option<String>,
    pub(crate) client_alias: Option<String>,
    pub(crate) machine_id: Option<String>,
    pub(crate) machine_alias: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudPairingTokenResponse {
    pub(crate) token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudRuntimeTokenResponse {
    pub(crate) token: String,
    pub(crate) expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudSessionInviteResponse {
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
pub(crate) struct CloudSessionInviteDetailsResponse {
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
pub(crate) struct CloudSessionInviteAcceptanceResponse {
    session_id: String,
    account_id: String,
    user_id: String,
    invited_by_user_id: String,
    joined_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudSessionInviteRevokedResponse {
    pub(crate) invite_id: String,
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudSessionMembersResponse {
    pub(crate) session_id: String,
    pub(crate) members: Vec<CloudSessionMemberResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudSessionMemberResponse {
    user_id: String,
    email: String,
    display_name: Option<String>,
    invited_by_user_id: Option<String>,
    joined_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudCollaboratorsResponse {
    pub(crate) collaborators: Vec<CloudCollaboratorResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudCollaboratorResponse {
    user_id: String,
    email: String,
    display_name: Option<String>,
    last_collaborated_at: String,
    shared_session_count: u32,
}

pub(crate) fn cloud_session_invite_from_response(
    response: CloudSessionInviteResponse,
) -> CloudSessionInvite {
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

pub(crate) fn cloud_session_invite_details_from_response(
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

pub(crate) fn cloud_session_invite_acceptance_from_response(
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

pub(crate) fn cloud_session_member_from_response(
    response: CloudSessionMemberResponse,
) -> CloudSessionMember {
    CloudSessionMember {
        user_id: response.user_id,
        email: response.email,
        display_name: response.display_name,
        invited_by_user_id: response.invited_by_user_id,
        joined_at: response.joined_at,
    }
}

pub(crate) fn cloud_collaborator_from_response(
    response: CloudCollaboratorResponse,
) -> CloudCollaborator {
    CloudCollaborator {
        user_id: response.user_id,
        email: response.email,
        display_name: response.display_name,
        last_collaborated_at: response.last_collaborated_at,
        shared_session_count: response.shared_session_count,
    }
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

pub(crate) async fn issue_cloud_runtime_token(
    profile: &PersistedCloudRelayProfile,
    subject: &str,
    subject_kind: &str,
    allowed_targets: Option<Vec<String>>,
    client_id: Option<String>,
    machine_id: Option<String>,
    session_id: Option<String>,
) -> Result<CloudRuntimeTokenResponse, DaemonError> {
    let mut body = serde_json::Map::new();
    if let Some(machine_credential) = profile.machine_credential.clone() {
        body.insert(
            "machineCredential".to_string(),
            serde_json::Value::String(machine_credential),
        );
    } else if let Some(session_token) = profile.cloud_session_token.clone() {
        body.insert(
            "sessionToken".to_string(),
            serde_json::Value::String(session_token),
        );
    }
    body.insert(
        "accountId".to_string(),
        serde_json::Value::String(profile.account_id.clone()),
    );
    body.insert(
        "subject".to_string(),
        serde_json::Value::String(subject.to_string()),
    );
    body.insert(
        "subjectKind".to_string(),
        serde_json::Value::String(subject_kind.to_string()),
    );
    body.insert(
        "realmId".to_string(),
        serde_json::Value::String(profile.realm_id.clone()),
    );
    body.insert(
        "ttlMs".to_string(),
        serde_json::Value::Number(serde_json::Number::from(CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS)),
    );
    body.insert(
        "userId".to_string(),
        serde_json::Value::String(profile.user_id.clone()),
    );
    if let Some(allowed_targets) = allowed_targets {
        body.insert(
            "allowedTargets".to_string(),
            serde_json::to_value(allowed_targets).map_err(|error| DaemonError::LocalTransport {
                operation: "encode cloud relay token request",
                message: error.to_string(),
            })?,
        );
    }
    if let Some(client_id) = client_id {
        body.insert("clientId".to_string(), serde_json::Value::String(client_id));
    }
    if let Some(machine_id) = machine_id {
        body.insert(
            "machineId".to_string(),
            serde_json::Value::String(machine_id),
        );
    }
    if let Some(session_id) = session_id {
        body.insert(
            "sessionId".to_string(),
            serde_json::Value::String(session_id),
        );
    }
    post_cloud_json(
        profile.api_url.clone(),
        "/relay/token",
        serde_json::Value::Object(body),
    )
    .await
}

pub(crate) fn cloud_profile_from_persisted(
    profile: &PersistedCloudRelayProfile,
) -> CloudRelayProfile {
    CloudRelayProfile {
        api_url: profile.api_url.clone(),
        email: profile.email.clone(),
        account_id: profile.account_id.clone(),
        user_id: profile.user_id.clone(),
        account_slug: profile.account_slug.clone(),
        realm_id: profile.realm_id.clone(),
        relay_url: profile.relay_url.clone(),
        issuer_id: profile.issuer_id.clone(),
        client_id: profile.client_id.clone(),
        client_alias: profile.client_alias.clone(),
        machine_id: profile.machine_id.clone(),
        machine_alias: profile.machine_alias.clone(),
        machine_credential: profile.machine_credential.clone(),
        cloud_session_token: profile.cloud_session_token.clone(),
        cloud_session_expires_at_ms: profile.cloud_session_expires_at_ms,
        token_expires_at_ms: profile.token_expires_at_ms,
    }
}

pub(crate) fn normalize_cloud_api_url(api_url: &str) -> Result<String, DaemonError> {
    let normalized = api_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "normalize cloud relay api url",
            message: "api_url must not be empty".to_string(),
        });
    }
    Ok(normalized)
}

pub(crate) async fn post_cloud_json<T>(
    api_url: String,
    path: &'static str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || post_cloud_json_blocking(api_url, path, body))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "post cloud relay json",
            message: error.to_string(),
        })?
}

pub(crate) async fn post_cloud_json_dynamic<T>(
    api_url: String,
    path: String,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || post_cloud_json_blocking(api_url, &path, body))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "post cloud relay json",
            message: error.to_string(),
        })?
}

pub(crate) async fn get_cloud_json<T>(api_url: String, path: String) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || get_cloud_json_blocking(api_url, &path))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get cloud relay json",
            message: error.to_string(),
        })?
}

fn post_cloud_json_blocking<T>(
    api_url: String,
    path: &str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let response = ureq::post(&url)
        .set("content-type", "application/json")
        .send_string(&body.to_string())
        .map_err(cloud_transport_error)?;
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

fn get_cloud_json_blocking<T>(api_url: String, path: &str) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let response = ureq::get(&url).call().map_err(cloud_transport_error)?;
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

pub(crate) fn cloud_url_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
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

fn cloud_transport_error(error: ureq::Error) -> DaemonError {
    let message = match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            if body.is_empty() {
                format!("cloud relay request failed with {status}")
            } else if let Some(code) = cloud_api_error_code(&body) {
                format!("cloud relay request failed with {status}: cloud_api_code={code}: {body}")
            } else {
                format!("cloud relay request failed with {status}: {body}")
            }
        }
        ureq::Error::Transport(error) => error.to_string(),
    };
    DaemonError::LocalTransport {
        operation: "cloud relay request",
        message,
    }
}

fn cloud_api_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|payload| {
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(|code| code.as_str())
                .map(str::to_string)
        })
}

pub(crate) fn is_stale_cloud_link_error(error: &DaemonError) -> bool {
    let message = match error {
        DaemonError::LocalTransport { message, .. } => message.as_str(),
        _ => return false,
    };
    [
        "cloud_api_code=session_invalid",
        "cloud_api_code=identity_revoked",
        "cloud_api_code=realm_not_found",
        "cloud_api_code=account_deleted",
        "cloud_api_code=user_deleted",
        "\"code\":\"session_invalid\"",
        "\"code\":\"identity_revoked\"",
        "\"code\":\"realm_not_found\"",
        "\"code\":\"account_deleted\"",
        "\"code\":\"user_deleted\"",
        "invalid_session",
        "cloud relay request failed with 401",
        "cloud relay request failed with 403",
    ]
    .iter()
    .any(|needle| message.contains(needle))
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
    fn cloud_url_component_percent_encodes_query_values() {
        assert_eq!(
            cloud_url_component("token/a+b?x=1"),
            "token%2Fa%2Bb%3Fx%3D1"
        );
        assert_eq!(cloud_url_component("abc-_.~XYZ"), "abc-_.~XYZ");
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

    #[test]
    fn cloud_api_error_code_reads_cloud_error_payloads() {
        assert_eq!(
            cloud_api_error_code(r#"{"error":{"code":"session_invalid"}}"#),
            Some("session_invalid".to_string())
        );
        assert_eq!(cloud_api_error_code(r#"{"error":{}}"#), None);
    }

    #[test]
    fn stale_cloud_link_errors_include_cloud_codes_and_auth_failures() {
        assert!(is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud_api_code=identity_revoked".to_string(),
        }));
        assert!(is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 401".to_string(),
        }));
        assert!(!is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "network timeout".to_string(),
        }));
    }
}
