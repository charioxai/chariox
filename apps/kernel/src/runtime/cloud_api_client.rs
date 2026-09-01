use serde::Deserialize;

use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::CloudRelayProfile;
use crate::runtime::cloud_relay_control::CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS;

mod http;
pub(crate) use http::{
    cloud_error_is_retryable, cloud_url_component, get_cloud_json, get_cloud_json_authenticated,
    is_stale_cloud_link_error, normalize_cloud_api_url, post_cloud_json,
    post_cloud_json_authenticated, post_cloud_json_dynamic,
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

pub(crate) const MANAGED_SLICE_RELAY_RUNTIME_ACTIONS: [&str; 5] = [
    "daemon.register",
    "daemon.heartbeat",
    "packet.route",
    "peer.request",
    "peer.event",
];
pub(crate) const MANAGED_SLICE_RELAY_RECOVERY_ACTIONS: [&str; 3] =
    ["daemon.register", "daemon.heartbeat", "peer.request"];
pub(crate) const MANAGED_SLICE_RELAY_BOOTSTRAP_TOKEN_TTL_MS: u64 = 30 * 60_000;
pub(crate) const MANAGED_SLICE_RELAY_DISCOVERY_TOKEN_TTL_MS: u64 = 60_000;
pub(crate) const MANAGED_SLICE_RELAY_RECOVERY_TOKEN_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Default)]
pub(crate) struct CloudRuntimeTokenRequestOptions {
    pub(crate) ttl_ms: Option<u64>,
    pub(crate) allowed_actions: Option<Vec<String>>,
    pub(crate) allowed_targets: Option<Vec<String>>,
    pub(crate) allow_unpaired_client_subject: bool,
    pub(crate) client_id: Option<String>,
    pub(crate) machine_id: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) public_key_thumbprint: Option<String>,
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
    options: CloudRuntimeTokenRequestOptions,
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
        serde_json::Value::Number(serde_json::Number::from(
            options.ttl_ms.unwrap_or(CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS),
        )),
    );
    body.insert(
        "userId".to_string(),
        serde_json::Value::String(profile.user_id.clone()),
    );
    if options.allow_unpaired_client_subject {
        body.insert(
            "allowUnpairedClientSubject".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    if let Some(allowed_actions) = options.allowed_actions {
        body.insert(
            "allowedActions".to_string(),
            serde_json::to_value(allowed_actions).map_err(|error| DaemonError::LocalTransport {
                operation: "encode cloud relay token request",
                message: error.to_string(),
            })?,
        );
    }
    if let Some(allowed_targets) = options.allowed_targets {
        body.insert(
            "allowedTargets".to_string(),
            serde_json::to_value(allowed_targets).map_err(|error| DaemonError::LocalTransport {
                operation: "encode cloud relay token request",
                message: error.to_string(),
            })?,
        );
    }
    if let Some(client_id) = options.client_id {
        body.insert("clientId".to_string(), serde_json::Value::String(client_id));
    }
    if let Some(machine_id) = options.machine_id {
        body.insert(
            "machineId".to_string(),
            serde_json::Value::String(machine_id),
        );
    }
    if let Some(session_id) = options.session_id {
        body.insert(
            "sessionId".to_string(),
            serde_json::Value::String(session_id),
        );
    }
    if let Some(public_key_thumbprint) = options.public_key_thumbprint {
        body.insert(
            "publicKeyThumbprint".to_string(),
            serde_json::Value::String(public_key_thumbprint),
        );
    }
    post_cloud_json(
        profile.api_url.clone(),
        "/relay/token",
        serde_json::Value::Object(body),
    )
    .await
}

pub(crate) async fn issue_cloud_slice_runtime_token(
    profile: &PersistedCloudRelayProfile,
    slice_kernel_ref: &str,
    owner_kernel_id: &str,
    worker_public_key: Option<&str>,
) -> Result<CloudRuntimeTokenResponse, DaemonError> {
    let machine_id = profile
        .machine_id
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "issue cloud slice relay token",
            message: "hosted slice relay requires a paired machine identity".to_string(),
        })?;
    if profile.machine_credential.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "issue cloud slice relay token",
            message: "hosted slice relay requires a machine credential".to_string(),
        });
    }
    let options = cloud_slice_runtime_token_options(machine_id, owner_kernel_id, worker_public_key);
    issue_cloud_runtime_token(profile, slice_kernel_ref, "kernel", options).await
}

pub(crate) async fn issue_cloud_slice_recovery_token(
    profile: &PersistedCloudRelayProfile,
    slice_kernel_ref: &str,
    owner_kernel_id: &str,
    worker_public_key: &str,
) -> Result<CloudRuntimeTokenResponse, DaemonError> {
    let machine_id = profile
        .machine_id
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "issue cloud slice recovery token",
            message: "hosted slice recovery requires a paired machine identity".to_string(),
        })?;
    if profile.machine_credential.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "issue cloud slice recovery token",
            message: "hosted slice recovery requires a machine credential".to_string(),
        });
    }
    issue_cloud_runtime_token(
        profile,
        slice_kernel_ref,
        "kernel",
        cloud_slice_recovery_token_options(machine_id, owner_kernel_id, worker_public_key),
    )
    .await
}

pub(crate) async fn issue_cloud_slice_discovery_token(
    profile: &PersistedCloudRelayProfile,
    owner_kernel_ref: &str,
    worker_kernel_ref: &str,
) -> Result<CloudRuntimeTokenResponse, DaemonError> {
    let machine_id = profile
        .machine_id
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "issue cloud slice discovery token",
            message: "hosted slice discovery requires a paired machine identity".to_string(),
        })?;
    if profile.machine_credential.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "issue cloud slice discovery token",
            message: "hosted slice discovery requires a machine credential".to_string(),
        });
    }
    issue_cloud_runtime_token(
        profile,
        &cloud_slice_discovery_subject(owner_kernel_ref, worker_kernel_ref),
        "client",
        cloud_slice_discovery_token_options(machine_id),
    )
    .await
}

pub(crate) async fn issue_cloud_relay_inventory_discovery_token(
    profile: &PersistedCloudRelayProfile,
    kernel_ref: &str,
) -> Result<CloudRuntimeTokenResponse, DaemonError> {
    let machine_id = profile
        .machine_id
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "issue cloud relay inventory discovery token",
            message: "hosted relay inventory discovery requires a paired machine identity"
                .to_string(),
        })?;
    if profile.machine_credential.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "issue cloud relay inventory discovery token",
            message: "hosted relay inventory discovery requires a machine credential".to_string(),
        });
    }
    issue_cloud_runtime_token(
        profile,
        &cloud_relay_inventory_discovery_subject(kernel_ref),
        "client",
        cloud_relay_inventory_discovery_token_options(machine_id),
    )
    .await
}

fn cloud_slice_discovery_subject(owner_kernel_ref: &str, worker_kernel_ref: &str) -> String {
    format!("slice-discovery:{owner_kernel_ref}:{worker_kernel_ref}")
}

fn cloud_slice_discovery_token_options(machine_id: String) -> CloudRuntimeTokenRequestOptions {
    CloudRuntimeTokenRequestOptions {
        ttl_ms: Some(MANAGED_SLICE_RELAY_DISCOVERY_TOKEN_TTL_MS),
        allowed_actions: Some(vec!["client.metadata.read".to_string()]),
        allow_unpaired_client_subject: true,
        machine_id: Some(machine_id),
        ..CloudRuntimeTokenRequestOptions::default()
    }
}

fn cloud_relay_inventory_discovery_subject(kernel_ref: &str) -> String {
    format!("relay-inventory:{kernel_ref}")
}

fn cloud_relay_inventory_discovery_token_options(
    machine_id: String,
) -> CloudRuntimeTokenRequestOptions {
    cloud_slice_discovery_token_options(machine_id)
}

fn cloud_slice_runtime_token_options(
    machine_id: String,
    owner_kernel_id: &str,
    worker_public_key: Option<&str>,
) -> CloudRuntimeTokenRequestOptions {
    if let Some(worker_public_key) = worker_public_key {
        CloudRuntimeTokenRequestOptions {
            allowed_actions: Some(
                MANAGED_SLICE_RELAY_RUNTIME_ACTIONS
                    .iter()
                    .map(|action| (*action).to_string())
                    .collect(),
            ),
            allowed_targets: Some(vec![owner_kernel_id.to_string()]),
            machine_id: Some(machine_id),
            public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                worker_public_key,
            )),
            ..CloudRuntimeTokenRequestOptions::default()
        }
    } else {
        // The unkeyed bootstrap token may only register and ask its owner for
        // the key-bound runtime token that replaces it.
        CloudRuntimeTokenRequestOptions {
            ttl_ms: Some(MANAGED_SLICE_RELAY_BOOTSTRAP_TOKEN_TTL_MS),
            allowed_actions: Some(vec![
                "daemon.register".to_string(),
                "daemon.heartbeat".to_string(),
                "peer.request".to_string(),
            ]),
            allowed_targets: Some(vec![owner_kernel_id.to_string()]),
            machine_id: Some(machine_id),
            ..CloudRuntimeTokenRequestOptions::default()
        }
    }
}

fn cloud_slice_recovery_token_options(
    machine_id: String,
    owner_kernel_id: &str,
    worker_public_key: &str,
) -> CloudRuntimeTokenRequestOptions {
    CloudRuntimeTokenRequestOptions {
        ttl_ms: Some(MANAGED_SLICE_RELAY_RECOVERY_TOKEN_TTL_MS),
        allowed_actions: Some(
            MANAGED_SLICE_RELAY_RECOVERY_ACTIONS
                .iter()
                .map(|action| (*action).to_string())
                .collect(),
        ),
        allowed_targets: Some(vec![owner_kernel_id.to_string()]),
        machine_id: Some(machine_id),
        public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
            worker_public_key,
        )),
        ..CloudRuntimeTokenRequestOptions::default()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_slice_active_token_request_matches_cloud_runtime_contract() {
        let options = cloud_slice_runtime_token_options(
            "machine-owner".to_string(),
            "kernel-owner",
            Some("worker-public-key"),
        );

        assert_eq!(
            options.allowed_actions,
            Some(
                MANAGED_SLICE_RELAY_RUNTIME_ACTIONS
                    .iter()
                    .map(|action| (*action).to_string())
                    .collect()
            )
        );
        assert_eq!(
            options.allowed_targets,
            Some(vec!["kernel-owner".to_string()])
        );
        assert_eq!(options.machine_id.as_deref(), Some("machine-owner"));
        assert!(!options.allow_unpaired_client_subject);
        assert_eq!(
            options.public_key_thumbprint,
            Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                "worker-public-key"
            ))
        );
    }

    #[test]
    fn managed_slice_bootstrap_token_outlives_the_bounded_broker_operation() {
        let options =
            cloud_slice_runtime_token_options("machine-owner".to_string(), "kernel-owner", None);

        assert_eq!(
            options.ttl_ms,
            Some(MANAGED_SLICE_RELAY_BOOTSTRAP_TOKEN_TTL_MS)
        );
        assert!(options.ttl_ms.unwrap() > 21 * 60_000);
        assert_eq!(
            options.allowed_actions,
            Some(vec![
                "daemon.register".to_string(),
                "daemon.heartbeat".to_string(),
                "peer.request".to_string(),
            ])
        );
        assert_eq!(
            options.allowed_targets,
            Some(vec!["kernel-owner".to_string()])
        );
        assert_eq!(options.public_key_thumbprint, None);
    }

    #[test]
    fn managed_slice_discovery_token_is_short_lived_and_metadata_only() {
        let options = cloud_slice_discovery_token_options("machine-owner".to_string());

        assert_eq!(
            options.ttl_ms,
            Some(MANAGED_SLICE_RELAY_DISCOVERY_TOKEN_TTL_MS)
        );
        assert_eq!(
            options.allowed_actions,
            Some(vec!["client.metadata.read".to_string()])
        );
        assert_eq!(options.machine_id.as_deref(), Some("machine-owner"));
        assert!(options.allow_unpaired_client_subject);
        assert_eq!(options.allowed_targets, None);
        assert_eq!(options.public_key_thumbprint, None);
    }

    #[test]
    fn managed_slice_discovery_subject_is_unique_per_worker() {
        assert_eq!(
            cloud_slice_discovery_subject("kernel-owner", "kernel-worker-a"),
            "slice-discovery:kernel-owner:kernel-worker-a"
        );
        assert_ne!(
            cloud_slice_discovery_subject("kernel-owner", "kernel-worker-a"),
            cloud_slice_discovery_subject("kernel-owner", "kernel-worker-b")
        );
    }

    #[test]
    fn hosted_relay_inventory_uses_a_short_lived_metadata_only_client() {
        let options = cloud_relay_inventory_discovery_token_options("machine-owner".to_string());

        assert_eq!(
            options.ttl_ms,
            Some(MANAGED_SLICE_RELAY_DISCOVERY_TOKEN_TTL_MS)
        );
        assert_eq!(
            options.allowed_actions,
            Some(vec!["client.metadata.read".to_string()])
        );
        assert_eq!(options.machine_id.as_deref(), Some("machine-owner"));
        assert!(options.allow_unpaired_client_subject);
        assert_eq!(options.allowed_targets, None);
        assert_eq!(options.public_key_thumbprint, None);
        assert_eq!(
            cloud_relay_inventory_discovery_subject("kernel-owner"),
            "relay-inventory:kernel-owner"
        );
    }

    #[test]
    fn managed_slice_recovery_token_is_long_lived_but_narrow() {
        let options = cloud_slice_recovery_token_options(
            "machine-owner".to_string(),
            "kernel-owner",
            "worker-public-key",
        );

        assert_eq!(
            options.ttl_ms,
            Some(MANAGED_SLICE_RELAY_RECOVERY_TOKEN_TTL_MS)
        );
        assert_eq!(
            options.allowed_actions,
            Some(
                MANAGED_SLICE_RELAY_RECOVERY_ACTIONS
                    .iter()
                    .map(|action| (*action).to_string())
                    .collect()
            )
        );
        assert_eq!(
            options.allowed_targets,
            Some(vec!["kernel-owner".to_string()])
        );
        assert_eq!(options.machine_id.as_deref(), Some("machine-owner"));
        assert!(options.public_key_thumbprint.is_some());
    }
}
