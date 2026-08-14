use std::collections::BTreeSet;

use chariox_event_protocol::{canonical_utc_timestamp, EventArtifact, PublishEventRequest};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct PublishEventBuilder {
    request: PublishEventRequest,
}

impl PublishEventBuilder {
    pub fn new(
        producer_id: impl Into<String>,
        event_interest_key: impl Into<String>,
        occurrence_id: impl Into<String>,
        event_type: impl Into<String>,
        occurred_at: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            request: PublishEventRequest {
                producer_id: producer_id.into(),
                event_interest_key: event_interest_key.into(),
                occurrence_id: occurrence_id.into(),
                event_type: event_type.into(),
                event_type_version: 1,
                occurred_at: occurred_at.into(),
                prompt: prompt.into(),
                artifacts: Vec::new(),
                metadata: Value::Null,
                reply_context: None,
                ttl_seconds: chariox_event_protocol::DEFAULT_EVENT_DELIVERY_TTL_SECONDS,
            },
        }
    }

    pub fn event_type_version(mut self, version: u32) -> Self {
        self.request.event_type_version = version;
        self
    }

    pub fn metadata(mut self, metadata: Value) -> Self {
        self.request.metadata = metadata;
        self
    }

    pub fn artifact(mut self, artifact: EventArtifact) -> Self {
        self.request.artifacts.push(artifact);
        self
    }

    pub fn ttl_seconds(mut self, ttl_seconds: u64) -> Self {
        self.request.ttl_seconds = ttl_seconds;
        self
    }

    pub fn build(mut self) -> Result<PublishEventRequest, String> {
        self.request.occurred_at = canonical_utc_timestamp(&self.request.occurred_at)?;
        self.request.validate()?;
        Ok(self.request)
    }
}

pub fn validate_manifest_envelope(manifest: &Value) -> Result<String, String> {
    let object = manifest
        .as_object()
        .ok_or_else(|| "event generator manifest must be an object".to_string())?;
    if object.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err("manifest schema_version must be 1".to_string());
    }
    for field in ["generator_id", "version", "name"] {
        require_manifest_string(object.get(field), field)?;
    }
    let protocol_version = object
        .get("protocol_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "manifest protocol_version is required".to_string())?;
    if protocol_version == 0 || protocol_version > super::AEGS_PROTOCOL_VERSION as u64 {
        return Err(format!(
            "manifest protocol_version {protocol_version} is newer than the supported protocol"
        ));
    }
    for registry_field in [
        "operator",
        "verification",
        "installed_count",
        "recommended",
        "availability",
        "manifest_digest",
    ] {
        if object.contains_key(registry_field) {
            return Err(format!(
                "manifest {registry_field} is registry metadata and must not be publisher-signed"
            ));
        }
    }
    let publisher = object
        .get("publisher")
        .and_then(Value::as_object)
        .ok_or_else(|| "manifest publisher must be an object".to_string())?;
    require_manifest_string(publisher.get("id"), "publisher.id")?;
    require_manifest_string(publisher.get("name"), "publisher.name")?;
    let events = object
        .get("events")
        .and_then(Value::as_array)
        .filter(|events| !events.is_empty() && events.len() <= 500)
        .ok_or_else(|| "manifest events must contain between 1 and 500 items".to_string())?;
    let mut event_keys = BTreeSet::new();
    for event in events {
        let event = event
            .as_object()
            .ok_or_else(|| "manifest event must be an object".to_string())?;
        let event_type = require_manifest_string(event.get("event_type"), "event.event_type")?;
        let version = event
            .get("version")
            .and_then(Value::as_u64)
            .filter(|version| *version > 0)
            .ok_or_else(|| "event.version must be greater than zero".to_string())?;
        require_manifest_string(event.get("name"), "event.name")?;
        if !event
            .get("filter_schema")
            .is_some_and(|value| value.is_object())
        {
            return Err("event.filter_schema must be an object".to_string());
        }
        if !event_keys.insert(format!("{event_type}@{version}")) {
            return Err(format!("duplicate manifest event {event_type}@{version}"));
        }
    }
    let signature = object
        .get("signature")
        .and_then(Value::as_object)
        .ok_or_else(|| "manifest signature must be an object".to_string())?;
    if signature.get("algorithm").and_then(Value::as_str) != Some("ed25519") {
        return Err("manifest signature algorithm must be ed25519".to_string());
    }
    require_manifest_string(signature.get("key_id"), "signature.key_id")?;
    require_manifest_string(signature.get("value"), "signature.value")?;
    let declared_digest = require_manifest_string(signature.get("digest"), "signature.digest")?;
    let computed_digest = unsigned_manifest_digest(manifest)?;
    if declared_digest != computed_digest {
        return Err(format!(
            "manifest digest mismatch: declared {declared_digest}, computed {computed_digest}"
        ));
    }
    Ok(computed_digest)
}

pub fn unsigned_manifest_digest(manifest: &Value) -> Result<String, String> {
    let mut unsigned = manifest.clone();
    unsigned
        .as_object_mut()
        .ok_or_else(|| "event generator manifest must be an object".to_string())?
        .remove("signature");
    let bytes =
        serde_json::to_vec(&canonical_json(&unsigned)).map_err(|error| error.to_string())?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn require_manifest_string<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str, String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("manifest {field} is required"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_dummy_fixture_passes_manifest_contract() {
        let manifest: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/event-generators/dummy/manifest.json"
        ))
        .unwrap();
        assert_eq!(
            validate_manifest_envelope(&manifest).unwrap(),
            manifest
                .pointer("/signature/digest")
                .and_then(Value::as_str)
                .unwrap()
        );
    }

    #[test]
    fn event_builder_enforces_protocol_limits() {
        let request = PublishEventBuilder::new(
            "dev.chariox.github",
            format!("sha256:{}", "a".repeat(64)),
            "delivery-1",
            "pull_request.opened",
            "2026-07-27T00:00:00Z",
            "Review the pull request.",
        )
        .metadata(serde_json::json!({"repository": "charioxai/chariox"}))
        .artifact(EventArtifact {
            name: "pull-request.json".to_string(),
            media_type: "application/json".to_string(),
            reference: "https://artifacts.example/pull-request.json".to_string(),
            size_bytes: None,
            digest: None,
        })
        .build()
        .unwrap();
        assert_eq!(request.event_type_version, 1);
        assert_eq!(request.artifacts.len(), 1);
    }

    #[test]
    fn event_builder_normalizes_provider_offsets_to_utc() {
        let request = PublishEventBuilder::new(
            "producer-1",
            "interest-1",
            "occurrence-1",
            "example.opened",
            "2026-01-15T14:00:00+02:00",
            "Handle the event",
        )
        .build()
        .unwrap();

        assert_eq!(request.occurred_at, "2026-01-15T12:00:00.000Z");
    }

    #[test]
    fn rejects_registry_operator_metadata_in_publisher_manifest() {
        let mut manifest: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/event-generators/dummy/manifest.json"
        ))
        .unwrap();
        manifest.as_object_mut().unwrap().insert(
            "operator".to_string(),
            serde_json::json!({"id": "hosted.chariox", "name": "Chariox hosted service"}),
        );
        assert!(validate_manifest_envelope(&manifest)
            .unwrap_err()
            .contains("operator is registry metadata"));
    }
}
