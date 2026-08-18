use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::validate_manifest_envelope;

pub const AEGS_CONFORMANCE_SUITE: &str = "chariox-aegs-conformance-v1";
pub const AEGS_CONFORMANCE_CHECKS: &[&str] = &[
    "identity",
    "manifest_signature",
    "webhook_normalization",
    "filters",
    "connection_lifecycle",
    "test_events",
    "provider_actions",
    "protocol_compatibility",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AegsConformanceAttestation {
    pub suite: String,
    pub result: String,
    pub checks: Vec<String>,
    pub manifest_digest: String,
    pub report_digest: String,
    pub event_protocol_version: u32,
    pub management_protocol_version: u32,
    pub completed_at_ms: u64,
}

pub fn create_conformance_attestation(
    report_bytes: &[u8],
    manifest: &Value,
    completed_at_ms: u64,
) -> Result<AegsConformanceAttestation, String> {
    if completed_at_ms == 0 {
        return Err("conformance completion time must be positive".to_string());
    }
    let report: Value = serde_json::from_slice(report_bytes)
        .map_err(|error| format!("conformance report is not valid JSON: {error}"))?;
    let report = report
        .as_object()
        .ok_or_else(|| "conformance report must be an object".to_string())?;
    if report.get("suite").and_then(Value::as_str) != Some(AEGS_CONFORMANCE_SUITE) {
        return Err(format!(
            "conformance report suite must be {AEGS_CONFORMANCE_SUITE}"
        ));
    }
    if report.get("passed").and_then(Value::as_bool) != Some(true) {
        return Err("conformance report must record passed=true".to_string());
    }
    let checks = report
        .get("checks")
        .and_then(Value::as_array)
        .ok_or_else(|| "conformance report must include checks".to_string())?;
    if checks.is_empty() || checks.len() > 64 {
        return Err("conformance report must contain between 1 and 64 checks".to_string());
    }
    let mut completed = BTreeSet::new();
    for check in checks {
        let check = check
            .as_str()
            .ok_or_else(|| "conformance report check names must be strings".to_string())?;
        if check.is_empty()
            || check.len() > 64
            || check
                .bytes()
                .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
        {
            return Err(
                "conformance report check names must be bounded lowercase identifiers".to_string(),
            );
        }
        if !completed.insert(check.to_string()) {
            return Err(format!("conformance report repeats check {check}"));
        }
    }
    let missing = AEGS_CONFORMANCE_CHECKS
        .iter()
        .filter(|required| !completed.contains(**required))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "conformance report is missing required checks: {}",
            missing.join(", ")
        ));
    }
    let manifest_digest = validate_manifest_envelope(manifest)?;
    Ok(AegsConformanceAttestation {
        suite: AEGS_CONFORMANCE_SUITE.to_string(),
        result: "passed".to_string(),
        checks: completed.into_iter().collect(),
        manifest_digest,
        report_digest: format!("sha256:{:x}", Sha256::digest(report_bytes)),
        event_protocol_version: chariox_event_protocol::EVENT_DELIVERY_PROTOCOL_VERSION,
        management_protocol_version: chariox_event_protocol::AEGS_MANAGEMENT_PROTOCOL_VERSION,
        completed_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign_manifest;
    use ed25519_dalek::SigningKey;

    #[test]
    fn attestation_binds_passing_report_to_manifest_and_protocols() {
        let mut unsigned: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/event-generators/dummy/manifest.json"
        ))
        .expect("fixture manifest");
        unsigned
            .as_object_mut()
            .expect("manifest object")
            .remove("signature");
        let manifest = sign_manifest(
            &unsigned,
            "com.example.signing.1".to_string(),
            &SigningKey::from_bytes(&[7; 32]),
        )
        .expect("signed manifest");
        let report = br#"{"suite":"chariox-aegs-conformance-v1","passed":true,"checks":["test_events","identity","provider_actions","manifest_signature","webhook_normalization","filters","connection_lifecycle","protocol_compatibility"]}"#;
        let attestation = create_conformance_attestation(report, &manifest, 1_700_000_000_000)
            .expect("attestation");
        assert_eq!(attestation.result, "passed");
        assert_eq!(attestation.checks.len(), AEGS_CONFORMANCE_CHECKS.len());
        assert_eq!(
            attestation.manifest_digest,
            manifest["signature"]["digest"].as_str().unwrap()
        );
        assert_eq!(attestation.event_protocol_version, 3);
        assert_eq!(attestation.management_protocol_version, 4);
        assert!(attestation.report_digest.starts_with("sha256:"));
    }

    #[test]
    fn attestation_rejects_failed_or_empty_reports() {
        let manifest = serde_json::json!({});
        for report in [
            br#"{"suite":"chariox-aegs-conformance-v1","passed":false,"checks":["identity"]}"#
                .as_slice(),
            br#"{"suite":"chariox-aegs-conformance-v1","passed":true,"checks":[]}"#.as_slice(),
        ] {
            assert!(create_conformance_attestation(report, &manifest, 1).is_err());
        }
    }

    #[test]
    fn attestation_rejects_incomplete_or_duplicate_check_matrices() {
        let manifest = serde_json::json!({});
        for report in [
            br#"{"suite":"chariox-aegs-conformance-v1","passed":true,"checks":["identity","manifest_signature"]}"#.as_slice(),
            br#"{"suite":"chariox-aegs-conformance-v1","passed":true,"checks":["identity","identity","manifest_signature","webhook_normalization","filters","connection_lifecycle","test_events","provider_actions","protocol_compatibility"]}"#.as_slice(),
        ] {
            assert!(create_conformance_attestation(report, &manifest, 1).is_err());
        }
    }
}
