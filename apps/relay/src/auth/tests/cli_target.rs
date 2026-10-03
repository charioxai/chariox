use super::*;

#[test]
fn generated_machine_client_command_passes_signed_relay_target_admission() {
    // The CLI tests generate this command through the real handler and parseArgs.
    // Re-sign its explicitly synthetic payload here to exercise relay admission.
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/machine-client-cli-relay-target.json"
    )))
    .expect("shared CLI target fixture must parse");
    assert_eq!(fixture["schemaVersion"], 1);
    let synthetic = fixture["syntheticToken"].as_str().unwrap();
    let (signing_input, signature) = synthetic.rsplit_once(".").unwrap();
    assert_eq!(signature, "synthetic-signature");
    let (_, payload) = signing_input.split_once(".").unwrap();
    let payload = URL_SAFE_NO_PAD.decode(payload).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&payload).unwrap(),
        fixture["claims"]
    );
    let claims = parse_cloud_jwt_claims(&payload).unwrap();
    let canonical = fixture["parsedTarget"]["daemonId"].as_str().unwrap();
    let alias = fixture["requestedAlias"].as_str().unwrap();
    assert!(fixture["parsedTarget"]["daemonAlias"].is_null());
    assert!(claims.allows_target(Some(canonical)));
    assert!(!claims.allows_target(Some(alias)));

    let issuer = fixture["claims"]["iss"].as_str().unwrap();
    let secret = b"synthetic-cli-relay-target-test-secret";
    let signature = URL_SAFE_NO_PAD.encode(sign_hmac(secret, signing_input.as_bytes()).unwrap());
    let token = format!("{signing_input}.{signature}");
    let verifier = RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([(
            issuer.to_owned(),
            String::from_utf8(secret.to_vec()).unwrap(),
        )]),
        fixture["validationTimeMs"].as_u64(),
    );
    verifier
        .verify(RelayAuthRequest {
            token: &token,
            action: RelayAction::ClientConnect,
            target: Some(canonical),
        })
        .expect("the generated canonical target must pass signed relay admission");
    for target in [Some(alias), Some("kernel-foreign")] {
        assert_eq!(
            verifier
                .verify(RelayAuthRequest {
                    token: &token,
                    action: RelayAction::ClientConnect,
                    target,
                })
                .unwrap_err(),
            RelayAuthError::TargetNotAllowed,
            "alias-only and foreign targets must remain denied"
        );
    }
}
