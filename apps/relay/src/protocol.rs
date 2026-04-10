use serde::{Deserialize, Serialize};

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
pub struct DaemonRegistration {
    pub auth_token: String,
    pub daemon_id: String,
    pub machine_id: String,
    #[serde(default)]
    pub daemon_alias: Option<String>,
    pub public_key: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
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
    },
    ClientConnect {
        auth_token: String,
        target: ClientTarget,
    },
    ClientConnected {
        target: ClientTarget,
        daemon_public_key: String,
    },
    ClientRequest {
        request_id: String,
        target: ClientTarget,
        encrypted_request: EncryptedRelayPayload,
    },
    DaemonRequest {
        relay_request_id: String,
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
        session_id: String,
        attachment_id: String,
        client_public_key: String,
        #[serde(default)]
        resume_from_event_id: Option<u64>,
    },
    DaemonUnsubscribe {
        relay_request_id: String,
        relay_subscription_id: String,
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
                daemon_alias: Some("mbp".to_string()),
                public_key: "public-key".to_string(),
                capabilities: vec!["kernel_ws".to_string()],
            },
        };
        let json = serde_json::to_string(&envelope).expect("envelope should serialize");
        assert!(json.contains("\"kind\":\"daemon_register\""));
        assert!(json.contains("\"daemon_id\":\"daemon-1\""));
        assert!(json.contains("\"public_key\":\"public-key\""));
    }
}
