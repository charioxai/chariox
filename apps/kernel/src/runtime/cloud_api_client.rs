use serde::Deserialize;

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::CloudRelayProfile;
use crate::runtime::cloud_relay_control::CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS;

mod http;
pub(crate) use http::{
    cloud_error_code, cloud_url_component, get_cloud_json, is_stale_cloud_link_error,
    normalize_cloud_api_url, post_cloud_json, post_cloud_json_dynamic,
};
mod session_collaboration;
pub(crate) use session_collaboration::{
    accept_cloud_session_invite, create_cloud_session_invite, list_cloud_collaborators,
    list_cloud_session_members, revoke_cloud_session_invite, show_cloud_session_invite,
};

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
pub(crate) struct CloudEventGeneratorManagementCapabilityResponse {
    pub(crate) token: String,
    pub(crate) expires_at: String,
}

pub(crate) async fn issue_event_generator_management_capability(
    profile: &PersistedCloudRelayProfile,
    kernel_id: &str,
    generator_id: &str,
    version: &str,
    manifest_digest: &str,
    management_url: &str,
) -> Result<CloudEventGeneratorManagementCapabilityResponse, DaemonError> {
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
    for (key, value) in [
        ("accountId", profile.account_id.as_str()),
        ("realmId", profile.realm_id.as_str()),
        ("kernelId", kernel_id),
        ("generatorId", generator_id),
        ("version", version),
        ("manifestDigest", manifest_digest),
        ("managementUrl", management_url),
    ] {
        body.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    if let Some(machine_id) = profile.machine_id.as_deref() {
        body.insert(
            "machineId".to_string(),
            serde_json::Value::String(machine_id.to_string()),
        );
    }
    body.insert(
        "userId".to_string(),
        serde_json::Value::String(profile.user_id.clone()),
    );
    post_cloud_json_dynamic(
        profile.api_url.clone(),
        format!(
            "/v1/event-generators/{}/management-capability",
            cloud_url_component(generator_id)
        ),
        serde_json::Value::Object(body),
    )
    .await
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
