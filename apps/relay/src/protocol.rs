use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayConnectionRole {
    Daemon,
    Client,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonRegistration {
    pub auth_token: String,
    pub daemon_id: String,
    pub machine_id: String,
    #[serde(default)]
    pub daemon_alias: Option<String>,
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
    ClientRequest {
        request_id: String,
        target: ClientTarget,
        request: Value,
    },
    DaemonResponse {
        request_id: String,
        response: Value,
    },
    ClientSubscribe {
        subscription_id: String,
        target: ClientTarget,
        stream: String,
        #[serde(default)]
        resume_from_event_id: Option<String>,
    },
    ClientUnsubscribe {
        subscription_id: String,
    },
    DaemonEvent {
        subscription_id: String,
        event_id: String,
        event: Value,
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
                capabilities: vec!["kernel_ws".to_string()],
            },
        };
        let json = serde_json::to_string(&envelope).expect("envelope should serialize");
        assert!(json.contains("\"kind\":\"daemon_register\""));
        assert!(json.contains("\"daemon_id\":\"daemon-1\""));
    }
}
