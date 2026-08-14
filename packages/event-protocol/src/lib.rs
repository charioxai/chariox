use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const EVENT_DELIVERY_PROTOCOL_VERSION: u32 = 2;
pub const AEGS_MANAGEMENT_PROTOCOL_VERSION: u32 = 3;
pub const DEFAULT_EVENT_DELIVERY_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const MAX_EVENT_PROMPT_BYTES: usize = 1024 * 1024;
pub const MAX_EVENT_ARTIFACTS: usize = 32;

/// Converts any RFC 3339 instant with an explicit offset into the UTC wire form.
pub fn canonical_utc_timestamp(value: &str) -> Result<String, String> {
    let instant = DateTime::parse_from_rfc3339(value.trim()).map_err(|_| {
        "timestamp must be an RFC 3339 instant with an explicit timezone".to_string()
    })?;
    Ok(instant
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// Validates the event protocol's UTC-on-the-wire timestamp rule.
pub fn validate_utc_timestamp(name: &str, value: &str) -> Result<(), String> {
    let normalized = value.trim();
    let instant = DateTime::parse_from_rfc3339(normalized)
        .map_err(|_| format!("{name} must be an RFC 3339 instant with an explicit timezone"))?;
    if !normalized.ends_with('Z') || instant.offset().local_minus_utc() != 0 {
        return Err(format!("{name} must use UTC with a Z suffix"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventArtifact {
    pub name: String,
    pub media_type: String,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl EventArtifact {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("artifact name", self.name.as_str()),
            ("artifact media_type", self.media_type.as_str()),
            ("artifact reference", self.reference.as_str()),
        ] {
            if value.trim().is_empty() || value.len() > 2048 {
                return Err(format!("{name} must contain between 1 and 2048 characters"));
            }
        }
        if self
            .digest
            .as_deref()
            .is_some_and(|digest| !valid_sha256_digest(digest))
        {
            return Err(
                "artifact digest must be sha256 followed by 64 lowercase hex characters"
                    .to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDeliveryEnvelope {
    pub delivery_id: String,
    pub binding_id: String,
    pub event_type: String,
    pub event_type_version: u32,
    pub occurrence_id: String,
    pub occurred_at: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<EventArtifact>,
    #[serde(default, skip_serializing_if = "is_json_null")]
    pub metadata: Value,
    pub expires_at_ms: u64,
}

impl EventDeliveryEnvelope {
    pub fn validate(&self, now_ms: u64) -> Result<(), String> {
        require_opaque_id("delivery_id", &self.delivery_id)?;
        require_opaque_id("binding_id", &self.binding_id)?;
        require_opaque_id("occurrence_id", &self.occurrence_id)?;
        if self.event_type.trim().is_empty() {
            return Err("event_type is required".to_string());
        }
        if self.event_type_version == 0 {
            return Err("event_type_version must be greater than zero".to_string());
        }
        validate_utc_timestamp("occurred_at", &self.occurred_at)?;
        if self.prompt.trim().is_empty() {
            return Err("prompt is required".to_string());
        }
        if self.prompt.len() > MAX_EVENT_PROMPT_BYTES {
            return Err(format!(
                "prompt exceeds the {MAX_EVENT_PROMPT_BYTES} byte workflow endpoint limit"
            ));
        }
        if self.artifacts.len() > MAX_EVENT_ARTIFACTS {
            return Err(format!(
                "artifacts exceeds the {MAX_EVENT_ARTIFACTS} item workflow endpoint limit"
            ));
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        if self.expires_at_ms <= now_ms {
            return Err("delivery has expired".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentRouteClaim {
    pub environment_id: String,
    pub event_interest_key: String,
    pub kernel_id: String,
    pub publication_id: String,
    pub binding_id: String,
    pub endpoint_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_ref: Option<String>,
    pub binding_revision: u64,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsSubscriptionClaim {
    pub binding_id: String,
    pub generator_id: String,
    pub connection_id: String,
    pub connection_scope: String,
    pub event_interest_key: String,
    pub event_type: String,
    pub event_type_version: u32,
    #[serde(default)]
    pub filter: Value,
    pub revision: u64,
    pub active: bool,
}

impl AegsSubscriptionClaim {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("binding_id", self.binding_id.as_str()),
            ("generator_id", self.generator_id.as_str()),
            ("connection_id", self.connection_id.as_str()),
            ("connection_scope", self.connection_scope.as_str()),
            ("event_interest_key", self.event_interest_key.as_str()),
            ("event_type", self.event_type.as_str()),
        ] {
            require_opaque_id(name, value)?;
        }
        if self.event_type_version == 0 {
            return Err("event_type_version must be greater than zero".to_string());
        }
        if self.revision == 0 {
            return Err("revision must be greater than zero".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsSubscriptionReconcileRequest {
    pub owner_id: String,
    pub generator_id: String,
    #[serde(default)]
    pub subscriptions: Vec<AegsSubscriptionClaim>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsSubscriptionReconcileResponse {
    pub accepted_binding_ids: Vec<String>,
    pub authoritative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsAuthorizationStartRequest {
    pub generator_id: String,
    pub owner_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_url: Option<String>,
}

impl AegsAuthorizationStartRequest {
    pub fn validate(&self) -> Result<(), String> {
        require_opaque_id("generator_id", &self.generator_id)?;
        require_opaque_id("owner_id", &self.owner_id)?;
        validate_return_url(self.return_url.as_deref())
    }
}

fn validate_return_url(return_url: Option<&str>) -> Result<(), String> {
    if return_url.is_some_and(|value| {
        value.len() > 2048
            || !(value.starts_with("https://")
                || value.starts_with("http://127.0.0.1:")
                || value.starts_with("http://localhost:"))
    }) {
        return Err("return_url must use HTTPS or loopback HTTP".to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsAuthorizationFlow {
    pub generator_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsProviderResource {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub connection_scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsProviderResourceQuery {
    pub generator_id: String,
    pub owner_id: String,
    pub connection_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub limit: u32,
}

impl AegsProviderResourceQuery {
    pub fn validate(&self) -> Result<(), String> {
        require_opaque_id("generator_id", &self.generator_id)?;
        require_opaque_id("owner_id", &self.owner_id)?;
        require_opaque_id("connection_id", &self.connection_id)?;
        if self.query.as_deref().is_some_and(|value| value.len() > 512) {
            return Err("resource query exceeds 512 characters".to_string());
        }
        if self
            .cursor
            .as_deref()
            .is_some_and(|value| value.len() > 2048)
        {
            return Err("resource cursor exceeds 2048 characters".to_string());
        }
        if !(1..=100).contains(&self.limit) {
            return Err("resource page limit must be between 1 and 100".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AegsConnectionStatus {
    Pending,
    Ready,
    Expired,
    Revoked,
    Unavailable,
    Error,
}

/// User-facing lifecycle state for an installed provider connection. This is deliberately
/// separate from `AegsConnectionStatus`: the latter is the provider record's wire status,
/// while this state describes the recovery or use action the kernel can present to a user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AegsConnectionLifecycleState {
    #[default]
    NotInstalled,
    AuthorizationRequired,
    Connecting,
    Connected,
    ConnectedRestricted,
    Degraded,
    ReauthorizationRequired,
    ProviderUnreachable,
    AegsUnavailable,
    Unused,
    Disconnecting,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionScope {
    pub id: String,
    pub label: String,
    pub granted: bool,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectedResource {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionInspectionRequest {
    pub generator_id: String,
    pub owner_id: String,
    pub connection_id: String,
}

impl AegsConnectionInspectionRequest {
    pub fn validate(&self) -> Result<(), String> {
        require_connection_identity(&self.generator_id, &self.owner_id, &self.connection_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionInspection {
    pub generator_id: String,
    pub connection_id: String,
    pub lifecycle_state: AegsConnectionLifecycleState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<AegsConnectionScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<AegsConnectedResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_health_check_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accepted_event_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<String>,
    #[serde(default)]
    pub test_event_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionRefreshRequest {
    pub generator_id: String,
    pub owner_id: String,
    pub connection_id: String,
}

impl AegsConnectionRefreshRequest {
    pub fn validate(&self) -> Result<(), String> {
        require_connection_identity(&self.generator_id, &self.owner_id, &self.connection_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionTestEventRequest {
    pub generator_id: String,
    pub owner_id: String,
    pub connection_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
}

impl AegsConnectionTestEventRequest {
    pub fn validate(&self) -> Result<(), String> {
        require_connection_identity(&self.generator_id, &self.owner_id, &self.connection_id)?;
        if self
            .event_type
            .as_deref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 512)
        {
            return Err("event_type must contain between 1 and 512 characters".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionTestEventResponse {
    pub occurrence_id: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn require_connection_identity(
    generator_id: &str,
    owner_id: &str,
    connection_id: &str,
) -> Result<(), String> {
    require_opaque_id("generator_id", generator_id)?;
    require_opaque_id("owner_id", owner_id)?;
    require_opaque_id("connection_id", connection_id)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegsConnectionSummary {
    pub generator_id: String,
    pub connection_id: String,
    pub status: AegsConnectionStatus,
    #[serde(default, skip_serializing_if = "is_json_null")]
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionQuery {
    pub generator_id: String,
    pub owner_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub limit: u32,
}

impl AegsConnectionQuery {
    pub fn validate(&self) -> Result<(), String> {
        require_opaque_id("generator_id", &self.generator_id)?;
        require_opaque_id("owner_id", &self.owner_id)?;
        if let Some(connection_id) = &self.connection_id {
            require_opaque_id("connection_id", connection_id)?;
        }
        if self
            .cursor
            .as_deref()
            .is_some_and(|value| value.len() > 2048)
        {
            return Err("connection cursor exceeds 2048 characters".to_string());
        }
        if !(1..=100).contains(&self.limit) {
            return Err("connection page limit must be between 1 and 100".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AegsConnectionPage {
    pub connections: Vec<AegsConnectionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionRevokeRequest {
    pub generator_id: String,
    pub owner_id: String,
    pub connection_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionRevokeResponse {
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsConnectionReconnectRequest {
    pub generator_id: String,
    pub owner_id: String,
    pub connection_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_url: Option<String>,
}

impl AegsConnectionReconnectRequest {
    pub fn validate(&self) -> Result<(), String> {
        require_opaque_id("generator_id", &self.generator_id)?;
        require_opaque_id("owner_id", &self.owner_id)?;
        require_opaque_id("connection_id", &self.connection_id)?;
        validate_return_url(self.return_url.as_deref())
    }
}

impl AegsConnectionRevokeRequest {
    pub fn validate(&self) -> Result<(), String> {
        require_opaque_id("generator_id", &self.generator_id)?;
        require_opaque_id("owner_id", &self.owner_id)?;
        require_opaque_id("connection_id", &self.connection_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AegsProviderResourcePage {
    pub resources: Vec<AegsProviderResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl EnvironmentRouteClaim {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("environment_id", self.environment_id.as_str()),
            ("event_interest_key", self.event_interest_key.as_str()),
            ("kernel_id", self.kernel_id.as_str()),
            ("publication_id", self.publication_id.as_str()),
            ("binding_id", self.binding_id.as_str()),
            ("endpoint_id", self.endpoint_id.as_str()),
        ] {
            require_opaque_id(name, value)?;
        }
        if self.binding_revision == 0 {
            return Err("binding_revision must be greater than zero".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelEnvironmentResume {
    pub environment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accepted_delivery_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<EnvironmentRouteClaim>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelToAedsMessage {
    Hello {
        protocol_version: u32,
        kernel_id: String,
        environments: Vec<KernelEnvironmentResume>,
    },
    ReconcileRoutes {
        environments: Vec<KernelEnvironmentResume>,
    },
    Ack {
        delivery_id: String,
    },
    Heartbeat {
        at_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AedsToKernelMessage {
    HelloAccepted {
        protocol_version: u32,
        heartbeat_interval_ms: u64,
    },
    RoutesReconciled {
        accepted_binding_ids: Vec<String>,
        conflicts: Vec<EventRouteConflict>,
    },
    Delivery {
        delivery: EventDeliveryEnvelope,
    },
    Heartbeat {
        at_ms: u64,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRouteConflict {
    pub environment_id: String,
    pub event_interest_key: String,
    pub requested_binding_id: String,
    pub existing_binding_id: String,
    pub existing_publication_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishEventRequest {
    pub producer_id: String,
    pub event_interest_key: String,
    pub occurrence_id: String,
    pub event_type: String,
    pub event_type_version: u32,
    pub occurred_at: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<EventArtifact>,
    #[serde(default, skip_serializing_if = "is_json_null")]
    pub metadata: Value,
    #[serde(default = "default_delivery_ttl_seconds")]
    pub ttl_seconds: u64,
}

impl PublishEventRequest {
    pub fn validate(&self) -> Result<(), String> {
        require_opaque_id("producer_id", &self.producer_id)?;
        require_opaque_id("event_interest_key", &self.event_interest_key)?;
        require_opaque_id("occurrence_id", &self.occurrence_id)?;
        if self.event_type.trim().is_empty() {
            return Err("event_type is required".to_string());
        }
        if self.event_type_version == 0 {
            return Err("event_type_version must be greater than zero".to_string());
        }
        validate_utc_timestamp("occurred_at", &self.occurred_at)?;
        if self.prompt.trim().is_empty() || self.prompt.len() > MAX_EVENT_PROMPT_BYTES {
            return Err("prompt is empty or exceeds the workflow endpoint limit".to_string());
        }
        if self.artifacts.len() > MAX_EVENT_ARTIFACTS {
            return Err("too many artifacts".to_string());
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        if self.ttl_seconds == 0 || self.ttl_seconds > 30 * 24 * 60 * 60 {
            return Err("ttl_seconds must be between 1 and 2592000".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishEventResponse {
    pub occurrence_id: String,
    pub accepted_route_count: usize,
    pub delivery_ids: Vec<String>,
    pub duplicate: bool,
}

pub fn event_interest_key(
    generator_id: &str,
    event_type: &str,
    event_type_version: u32,
    connection_scope: &str,
    canonical_filter: &Value,
) -> Result<String, serde_json::Error> {
    let value = serde_json::json!({
        "generator_id": generator_id.trim(),
        "event_type": event_type.trim(),
        "event_type_version": event_type_version,
        "connection_scope": connection_scope.trim(),
        "filter": canonical_json(canonical_filter),
    });
    let encoded = serde_json::to_vec(&value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical_json(&values[key])))
                    .collect(),
            )
        }
        value => value.clone(),
    }
}

fn require_opaque_id(name: &str, value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 {
        return Err(format!("{name} must contain between 1 and 512 characters"));
    }
    Ok(())
}

fn valid_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn default_delivery_ttl_seconds() -> u64 {
    DEFAULT_EVENT_DELIVERY_TTL_SECONDS
}

fn is_json_null(value: &Value) -> bool {
    value.is_null()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interest_key_is_stable_across_object_key_order() {
        let left = event_interest_key(
            "github",
            "pull_request.opened",
            1,
            "installation:1",
            &serde_json::json!({"repository": "chariox", "owner": "example"}),
        )
        .unwrap();
        let right = event_interest_key(
            "github",
            "pull_request.opened",
            1,
            "installation:1",
            &serde_json::json!({"owner": "example", "repository": "chariox"}),
        )
        .unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn delivery_rejects_an_expired_envelope() {
        let delivery = EventDeliveryEnvelope {
            delivery_id: "delivery-1".to_string(),
            binding_id: "binding-1".to_string(),
            event_type: "test.event".to_string(),
            event_type_version: 1,
            occurrence_id: "occurrence-1".to_string(),
            occurred_at: "2026-07-27T00:00:00Z".to_string(),
            prompt: "Handle the test event.".to_string(),
            artifacts: Vec::new(),
            metadata: Value::Null,
            expires_at_ms: 9,
        };
        assert!(delivery.validate(10).unwrap_err().contains("expired"));
    }

    #[test]
    fn event_timestamps_are_absolute_and_utc_on_the_wire() {
        assert_eq!(
            canonical_utc_timestamp("2026-01-15T14:00:00+02:00").unwrap(),
            "2026-01-15T12:00:00.000Z"
        );
        assert!(validate_utc_timestamp("occurred_at", "2026-01-15T12:00:00Z").is_ok());
        assert!(validate_utc_timestamp("occurred_at", "2026-01-15T14:00:00+02:00").is_err());
        assert!(canonical_utc_timestamp("2026-01-15T12:00:00").is_err());
    }

    #[test]
    fn artifacts_require_retrievable_reference_metadata() {
        let valid = EventArtifact {
            name: "pull-request.json".to_string(),
            media_type: "application/json".to_string(),
            reference: "https://artifacts.example/pull-request.json".to_string(),
            size_bytes: Some(42),
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        };
        assert!(valid.validate().is_ok());
        let mut invalid = valid;
        invalid.reference.clear();
        assert!(invalid.validate().unwrap_err().contains("reference"));
    }

    #[test]
    fn aegs_management_requests_are_bounded_and_require_safe_return_urls() {
        let authorization = AegsAuthorizationStartRequest {
            generator_id: "dev.chariox.github".to_string(),
            owner_id: "owner-kernel-user".to_string(),
            return_url: Some("http://provider.example/callback".to_string()),
        };
        assert!(authorization.validate().unwrap_err().contains("HTTPS"));

        let resources = AegsProviderResourceQuery {
            generator_id: "dev.chariox.github".to_string(),
            owner_id: "owner-kernel-user".to_string(),
            connection_id: "connection-1".to_string(),
            query: None,
            cursor: None,
            limit: 101,
        };
        assert!(resources
            .validate()
            .unwrap_err()
            .contains("between 1 and 100"));

        let connections = AegsConnectionQuery {
            generator_id: "dev.chariox.github".to_string(),
            owner_id: "owner-kernel-user".to_string(),
            connection_id: None,
            cursor: None,
            limit: 0,
        };
        assert!(connections
            .validate()
            .unwrap_err()
            .contains("between 1 and 100"));

        let inspection = AegsConnectionInspectionRequest {
            generator_id: "dev.chariox.github".to_string(),
            owner_id: "owner-kernel-user".to_string(),
            connection_id: "connection-1".to_string(),
        };
        assert!(inspection.validate().is_ok());
        let invalid_test = AegsConnectionTestEventRequest {
            generator_id: inspection.generator_id,
            owner_id: inspection.owner_id,
            connection_id: inspection.connection_id,
            event_type: Some(String::new()),
        };
        assert!(invalid_test.validate().unwrap_err().contains("event_type"));
    }
}
