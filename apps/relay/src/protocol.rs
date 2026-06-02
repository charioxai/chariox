use serde::{Deserialize, Serialize};

use crate::auth::{RelaySubjectKind, VerifiedRelayIdentity};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayConnectionRole {
    Daemon,
    Client,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedRelayPayload {
    pub sender_public_key: String,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayCallerIdentity {
    pub realm_id: String,
    pub subject: String,
    pub subject_kind: RelaySubjectKind,
    #[serde(default)]
    pub expires_at_ms: u64,
    #[serde(default)]
    pub token_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
}

impl From<VerifiedRelayIdentity> for RelayCallerIdentity {
    fn from(identity: VerifiedRelayIdentity) -> Self {
        Self {
            realm_id: identity.realm_id,
            subject: identity.subject,
            subject_kind: identity.subject_kind,
            expires_at_ms: identity.expires_at_ms,
            token_id: identity.token_id,
            user_id: identity.user_id,
            public_key_thumbprint: identity.public_key_thumbprint,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonRegistration {
    pub auth_token: String,
    pub daemon_id: String,
    pub machine_id: String,
    #[serde(default)]
    pub machine_alias: Option<String>,
    #[serde(default)]
    pub os_name: Option<String>,
    #[serde(default)]
    pub kernel_started_at_ms: u64,
    #[serde(default)]
    pub daemon_alias: Option<String>,
    #[serde(default)]
    pub kernel_alias: Option<String>,
    pub public_key: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub available_providers: Vec<String>,
    #[serde(default)]
    pub provider_accounts: Vec<RelayProviderAccountSummary>,
    #[serde(default)]
    pub accepting_remote_leases: bool,
    #[serde(default)]
    pub leased_agent_count: u32,
    #[serde(default)]
    pub local_session_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayMachinePresence {
    pub machine_id: String,
    #[serde(default)]
    pub machine_alias: Option<String>,
    pub kernel_count: usize,
    #[serde(default)]
    pub available_providers: Vec<String>,
    #[serde(default)]
    pub provider_accounts: Vec<RelayProviderAccountSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayKernelPresence {
    pub kernel_id: String,
    pub machine_id: String,
    #[serde(default)]
    pub machine_alias: Option<String>,
    #[serde(default)]
    pub relay_alias: Option<String>,
    #[serde(default)]
    pub kernel_alias: Option<String>,
    #[serde(default)]
    pub available_providers: Vec<String>,
    #[serde(default)]
    pub provider_accounts: Vec<RelayProviderAccountSummary>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub accepting_remote_leases: bool,
    #[serde(default)]
    pub leased_agent_count: u32,
    #[serde(default)]
    pub local_session_count: u32,
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProviderAccountSummary {
    pub provider: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayMetadataQuery {
    ListLiveMachines,
    ListLiveKernelsForMachine { machine_ref: String },
    GetLiveKernel { kernel_ref: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTarget {
    #[serde(default)]
    pub daemon_id: Option<String>,
    #[serde(default)]
    pub daemon_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayEnvelope {
    DaemonRegister {
        registration: DaemonRegistration,
    },
    DaemonHeartbeat {
        daemon_id: String,
        #[serde(default)]
        registration: Option<DaemonRegistration>,
    },
    ClientConnect {
        auth_token: String,
        target: ClientTarget,
    },
    ClientConnected {
        target: ClientTarget,
        daemon_public_key: String,
    },
    ClientMetadataRequest {
        request_id: String,
        auth_token: String,
        query: RelayMetadataQuery,
    },
    ClientMetadataResponse {
        request_id: String,
        machines: Option<Vec<RelayMachinePresence>>,
        kernels: Option<Vec<RelayKernelPresence>>,
        kernel: Option<RelayKernelPresence>,
        error: Option<RelayError>,
    },
    DaemonPeerRequest {
        request_id: String,
        target: ClientTarget,
        encrypted_request: EncryptedRelayPayload,
    },
    DaemonIncomingPeerRequest {
        relay_request_id: String,
        from_daemon_id: String,
        #[serde(default)]
        caller_identity: Option<RelayCallerIdentity>,
        encrypted_request: EncryptedRelayPayload,
    },
    DaemonIncomingPeerResponse {
        relay_request_id: String,
        encrypted_response: Option<EncryptedRelayPayload>,
        error: Option<RelayError>,
    },
    DaemonPeerResponse {
        request_id: String,
        from_daemon_id: String,
        encrypted_response: Option<EncryptedRelayPayload>,
        error: Option<RelayError>,
    },
    DaemonPeerEvent {
        target: ClientTarget,
        encrypted_event: EncryptedRelayPayload,
    },
    DaemonIncomingPeerEvent {
        from_daemon_id: String,
        #[serde(default)]
        caller_identity: Option<RelayCallerIdentity>,
        encrypted_event: EncryptedRelayPayload,
    },
    ClientRequest {
        request_id: String,
        target: ClientTarget,
        encrypted_request: EncryptedRelayPayload,
    },
    DaemonRequest {
        relay_request_id: String,
        #[serde(default)]
        caller_identity: Option<RelayCallerIdentity>,
        encrypted_request: EncryptedRelayPayload,
    },
    DaemonResponse {
        relay_request_id: String,
        encrypted_response: Option<EncryptedRelayPayload>,
        error: Option<RelayError>,
    },
    ClientResponse {
        request_id: String,
        encrypted_response: Option<EncryptedRelayPayload>,
        error: Option<RelayError>,
    },
    ClientSubscribe {
        request_id: String,
        subscription_id: String,
        target: ClientTarget,
        session_id: String,
        attachment_id: String,
        client_public_key: String,
        #[serde(default)]
        subscription_scope: Option<String>,
        #[serde(default)]
        resume_from_event_id: Option<u64>,
    },
    ClientUnsubscribe {
        request_id: String,
        subscription_id: String,
        client_public_key: String,
    },
    DaemonSubscribe {
        relay_request_id: String,
        relay_subscription_id: String,
        #[serde(default)]
        caller_identity: Option<RelayCallerIdentity>,
        session_id: String,
        attachment_id: String,
        client_public_key: String,
        #[serde(default)]
        subscription_scope: Option<String>,
        #[serde(default)]
        resume_from_event_id: Option<u64>,
    },
    DaemonUnsubscribe {
        relay_request_id: String,
        relay_subscription_id: String,
        #[serde(default)]
        caller_identity: Option<RelayCallerIdentity>,
        client_public_key: String,
    },
    DaemonEvent {
        subscription_id: String,
        event_id: u64,
        encrypted_event: EncryptedRelayPayload,
    },
    ClientEvent {
        subscription_id: String,
        event_id: u64,
        encrypted_event: EncryptedRelayPayload,
    },
    Close {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_daemon_registration_envelope() {
        let envelope = RelayEnvelope::DaemonRegister {
            registration: DaemonRegistration {
                auth_token: "secret".to_string(),
                daemon_id: "daemon-1".to_string(),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("workstation".to_string()),
                os_name: Some("macOS".to_string()),
                kernel_started_at_ms: 10,
                daemon_alias: Some("mbp".to_string()),
                kernel_alias: Some("default".to_string()),
                public_key: "public-key".to_string(),
                capabilities: vec!["kernel_ws".to_string()],
                available_providers: vec!["opencode".to_string()],
                provider_accounts: Vec::new(),
                accepting_remote_leases: false,
                leased_agent_count: 0,
                local_session_count: 1,
            },
        };
        let json = serde_json::to_string(&envelope).expect("envelope should serialize");
        assert!(json.contains("\"kind\":\"daemon_register\""));
        assert!(json.contains("\"daemon_id\":\"daemon-1\""));
        assert!(json.contains("\"public_key\":\"public-key\""));
    }
}
