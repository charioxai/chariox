use chariox_aegs_sdk::{
    AegsConformanceAttestation, CreateAuthorizationRequest, AEGS_CONFORMANCE_CHECKS,
};

#[test]
fn create_authorization_request_is_public() {
    let request = CreateAuthorizationRequest {
        state_digest: "state",
        connection_id: "connection",
        owner_id: "owner",
        provider: "provider",
        return_url: Some("https://example.com/callback"),
        expires_at_ms: 2,
        now_ms: 1,
    };

    assert_eq!(request.connection_id, "connection");
}

#[test]
fn conformance_attestation_is_public() {
    let attestation = AegsConformanceAttestation {
        suite: "chariox-aegs-conformance-v1".to_string(),
        result: "passed".to_string(),
        checks: AEGS_CONFORMANCE_CHECKS
            .iter()
            .map(|check| (*check).to_string())
            .collect(),
        manifest_digest: "sha256:manifest".to_string(),
        report_digest: "sha256:report".to_string(),
        event_protocol_version: 3,
        management_protocol_version: 4,
        completed_at_ms: 1,
    };

    assert_eq!(attestation.management_protocol_version, 4);
    assert_eq!(attestation.checks.len(), 8);
}
