use std::collections::BTreeSet;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chariox_event_protocol::{canonical_utc_timestamp, EventArtifact, PublishEventRequest};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
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
            .filter(|version| *version > 0 && *version <= u32::MAX as u64)
            .ok_or_else(|| "event.version must be a positive u32".to_string())?;
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
    let bytes = serde_json_canonicalizer::to_vec(&unsigned).map_err(|error| error.to_string())?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

/// Signs the canonical unsigned manifest and returns a publisher-ready envelope.
///
/// The signature covers canonical unsigned JSON, rather than a serialization
/// chosen by a caller. The digest is retained in the envelope as a cheap
/// integrity check. This keeps registry verification independent of JSON key
/// ordering. Publisher keys never leave the caller's process.
pub fn sign_manifest(
    unsigned_manifest: &Value,
    key_id: impl Into<String>,
    signing_key: &SigningKey,
) -> Result<Value, String> {
    let mut manifest = unsigned_manifest.clone();
    manifest
        .as_object_mut()
        .ok_or_else(|| "event generator manifest must be an object".to_string())?
        .remove("signature");
    let digest = unsigned_manifest_digest(&manifest)?;
    let bytes = canonical_manifest_bytes(&manifest)?;
    let signature = signing_key.sign(&bytes);
    manifest
        .as_object_mut()
        .expect("manifest object was checked above")
        .insert(
            "signature".to_string(),
            serde_json::json!({
                "key_id": key_id.into(),
                "algorithm": "ed25519",
                "digest": digest,
                "value": BASE64.encode(signature.to_bytes()),
            }),
        );
    validate_manifest_envelope(&manifest)?;
    Ok(manifest)
}

/// Verifies the signature in a manifest against a trusted publisher key.
/// Registry metadata must decide which key IDs are trusted; this function does
/// not fetch keys or make network requests.
pub fn verify_manifest_signature(
    manifest: &Value,
    verifying_key: &VerifyingKey,
) -> Result<String, String> {
    let digest = validate_manifest_envelope(manifest)?;
    let value = manifest
        .pointer("/signature/value")
        .and_then(Value::as_str)
        .ok_or_else(|| "manifest signature.value is required".to_string())?;
    let bytes = BASE64
        .decode(value)
        .map_err(|error| format!("manifest signature.value is not valid base64: {error}"))?;
    let signature = Signature::from_slice(&bytes).map_err(|error| {
        format!("manifest signature.value is not a valid ed25519 signature: {error}")
    })?;
    let bytes = canonical_manifest_bytes(&without_signature(manifest)?)?;
    verifying_key
        .verify_strict(&bytes, &signature)
        .map_err(|_| "manifest signature verification failed".to_string())?;
    Ok(digest)
}

fn without_signature(manifest: &Value) -> Result<Value, String> {
    let mut unsigned = manifest.clone();
    unsigned
        .as_object_mut()
        .ok_or_else(|| "event generator manifest must be an object".to_string())?
        .remove("signature");
    Ok(unsigned)
}

fn canonical_manifest_bytes(manifest: &Value) -> Result<Vec<u8>, String> {
    serde_json_canonicalizer::to_vec(manifest).map_err(|error| error.to_string())
}

/// Parses a raw 32-byte Ed25519 private key from hexadecimal or base64 text.
/// This intentionally accepts only raw keys, not PEM, so accidental upload of a
/// certificate or a multiline secret fails before signing.
pub fn parse_signing_key(value: &str) -> Result<SigningKey, String> {
    let trimmed = value.trim();
    let bytes = if trimmed.len() == 64 && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        (0..trimmed.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&trimmed[index..index + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("invalid hexadecimal signing key: {error}"))?
    } else {
        BASE64
            .decode(trimmed)
            .map_err(|error| format!("signing key must be raw 32-byte hex or base64: {error}"))?
    };
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "signing key must contain exactly 32 bytes".to_string())?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn require_manifest_string<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str, String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("manifest {field} is required"))
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

    #[test]
    fn signs_and_verifies_canonical_manifest() {
        let mut manifest: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/event-generators/dummy/manifest.json"
        ))
        .unwrap();
        manifest.as_object_mut().unwrap().remove("signature");
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let signed = sign_manifest(&manifest, "test.key", &signing_key).unwrap();
        assert_eq!(
            verify_manifest_signature(&signed, &signing_key.verifying_key()).unwrap(),
            unsigned_manifest_digest(&signed).unwrap()
        );
    }

    #[test]
    fn rejects_tampered_signature_even_when_digest_is_unchanged() {
        let mut manifest: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/event-generators/dummy/manifest.json"
        ))
        .unwrap();
        manifest["signature"]["value"] = Value::String("not-a-signature".to_string());
        let key = SigningKey::from_bytes(&[7; 32]);
        assert!(verify_manifest_signature(&manifest, &key.verifying_key()).is_err());
        assert!(validate_manifest_envelope(&manifest).is_ok());
    }

    #[test]
    fn parses_hex_and_base64_signing_keys() {
        let hex = "07".repeat(32);
        let base64 = BASE64.encode([7; 32]);
        assert_eq!(parse_signing_key(&hex).unwrap().to_bytes(), [7; 32]);
        assert_eq!(parse_signing_key(&base64).unwrap().to_bytes(), [7; 32]);
    }

    #[test]
    fn uses_rfc8785_number_and_string_vectors() {
        let value = serde_json::json!({
            "\u{e9}": "\u{2028}",
            "numbers": [1e-6, 1e23, -0.0],
        });
        let bytes = canonical_manifest_bytes(&value).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"numbers":[0.000001,1e+23,0],"é":" "}"#
        );
    }

    #[test]
    fn published_fixture_digest_is_interoperable() {
        let manifest: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/event-generators/dummy/manifest.json"
        ))
        .unwrap();
        assert_eq!(
            unsigned_manifest_digest(&manifest).unwrap(),
            manifest
                .pointer("/signature/digest")
                .unwrap()
                .as_str()
                .unwrap()
        );
    }
}
