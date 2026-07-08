use super::*;

#[test]
fn shared_verifier_accepts_matching_token() {
    let verifier = RelayAuthVerifier::shared(Some("secret".to_string()));
    let identity = verifier
        .verify(RelayAuthRequest {
            token: "secret",
            action: RelayAction::ClientConnect,
            target: Some("daemon-1"),
        })
        .expect("matching shared token should be accepted");

    assert_eq!(identity.realm_id, DEFAULT_RELAY_REALM_ID);
    assert_eq!(identity.subject_kind, RelaySubjectKind::Client);
    assert_eq!(identity.user_id.as_deref(), Some("local"));
}

#[test]
fn shared_verifier_rejects_mismatched_token() {
    let verifier = RelayAuthVerifier::shared(Some("secret".to_string()));
    let error = verifier
        .verify(RelayAuthRequest {
            token: "wrong",
            action: RelayAction::DaemonRegister,
            target: None,
        })
        .expect_err("mismatched shared token should be rejected");

    assert_eq!(error, RelayAuthError::InvalidToken);
}

#[test]
fn shared_verifier_preserves_open_bootstrap_mode_when_no_token_configured() {
    let verifier = RelayAuthVerifier::shared(None);
    let identity = verifier
        .verify(RelayAuthRequest {
            token: "",
            action: RelayAction::DaemonRegister,
            target: None,
        })
        .expect("unconfigured shared-token mode should preserve current open behavior");

    assert_eq!(identity.realm_id, DEFAULT_RELAY_REALM_ID);
    assert_eq!(identity.subject_kind, RelaySubjectKind::Kernel);
}

#[test]
fn claims_check_actions_and_targets() {
    let claims = RelayTokenClaims {
        issuer: "issuer".to_string(),
        subject: "client-1".to_string(),
        subject_kind: RelaySubjectKind::Client,
        realm_id: "realm-1".to_string(),
        allowed_actions: vec![RelayAction::ClientConnect],
        allowed_targets: Some(vec!["daemon-1".to_string()]),
        issued_at_ms: 10,
        expires_at_ms: 20,
        token_id: "token-1".to_string(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: None,
        client_id: Some("client-1".to_string()),
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    };

    assert!(claims.allows_action(RelayAction::ClientConnect));
    assert!(!claims.allows_action(RelayAction::DaemonRegister));
    assert!(claims.allows_target(Some("daemon-1")));
    assert!(!claims.allows_target(Some("daemon-2")));
    assert!(!claims.allows_target(None));
}

#[test]
fn clock_failure_maps_to_end_of_time_so_expiry_fails_closed() {
    let clock_error = UNIX_EPOCH
        .duration_since(SystemTime::now())
        .expect_err("epoch should be before now");
    assert_eq!(fail_closed_unix_ms(Err(clock_error)), u64::MAX);

    let claims = RelayTokenClaims {
        issuer: "issuer".to_string(),
        subject: "client-1".to_string(),
        subject_kind: RelaySubjectKind::Client,
        realm_id: "realm-1".to_string(),
        allowed_actions: vec![RelayAction::ClientConnect],
        allowed_targets: None,
        issued_at_ms: 10,
        expires_at_ms: 20,
        token_id: "token-1".to_string(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: None,
        client_id: None,
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    };
    assert_eq!(
        validate_claims(&claims, RelayAction::ClientConnect, None, Some(u64::MAX)),
        Err(RelayAuthError::TokenExpired)
    );
}

#[test]
fn scoped_verifier_fails_closed_without_tokens_or_issuers() {
    let verifier = RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::default());
    let error = verifier
        .verify(RelayAuthRequest {
            token: "scoped-token",
            action: RelayAction::ClientConnect,
            target: Some("daemon-1"),
        })
        .expect_err("scoped verifier should fail closed without issuer metadata");

    assert_eq!(error, RelayAuthError::ScopedTokensUnavailable);
}

#[test]
fn scoped_verifier_checks_action_target_and_expiration() {
    let mut tokens = BTreeMap::new();
    tokens.insert(
        "client-token".to_string(),
        RelayTokenClaims {
            issuer: "issuer".to_string(),
            subject: "client-1".to_string(),
            subject_kind: RelaySubjectKind::Client,
            realm_id: "realm-1".to_string(),
            allowed_actions: vec![RelayAction::ClientConnect],
            allowed_targets: Some(vec!["daemon-1".to_string()]),
            issued_at_ms: 10,
            expires_at_ms: 20,
            token_id: "token-1".to_string(),
            account_id: None,
            organization_id: None,
            user_id: Some("user-1".to_string()),
            device_id: None,
            machine_id: None,
            client_id: Some("client-1".to_string()),
            session_id: None,
            public_key_thumbprint: Some("thumbprint".to_string()),
            entitlements_version: None,
        },
    );
    let verifier =
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(tokens, BTreeMap::new(), Some(15)));

    let identity = verifier
        .verify(RelayAuthRequest {
            token: "client-token",
            action: RelayAction::ClientConnect,
            target: Some("daemon-1"),
        })
        .expect("valid scoped token should verify");
    assert_eq!(identity.realm_id, "realm-1");
    assert_eq!(identity.token_id.as_deref(), Some("token-1"));
    assert_eq!(
        identity.public_key_thumbprint.as_deref(),
        Some("thumbprint")
    );

    let action_error = verifier
        .verify(RelayAuthRequest {
            token: "client-token",
            action: RelayAction::DaemonRegister,
            target: Some("daemon-1"),
        })
        .expect_err("wrong action should be rejected");
    assert_eq!(action_error, RelayAuthError::ActionNotAllowed);

    let target_error = verifier
        .verify(RelayAuthRequest {
            token: "client-token",
            action: RelayAction::ClientConnect,
            target: Some("daemon-2"),
        })
        .expect_err("wrong target should be rejected");
    assert_eq!(target_error, RelayAuthError::TargetNotAllowed);
}

// Cross-implementation conformance: this JWT was minted by the
// arroba-cloud @arroba-cloud/relay-tokens issuer (the canonical issuer)
// with the fixed secret/timestamps below. The identical fixture is checked
// by the TypeScript suite; if either side's claim wire shape drifts, one
// of the two conformance tests fails. Keep in sync with
// arroba-cloud/packages/relay-tokens/src/conformance-vectors.json.
const CONFORMANCE_SECRET: &str = "conformance-shared-secret";
const CONFORMANCE_ISSUER: &str = "arroba-cloud-conformance";
const CONFORMANCE_TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJhcnJvYmEtY2xvdWQtY29uZm9ybWFuY2UiLCJzdWIiOiJjbGllbnQtMSIsInJlYWxtX2lkIjoicmVhbG0tMSIsInN1YmplY3Rfa2luZCI6ImNsaWVudCIsImFsbG93ZWRfYWN0aW9ucyI6WyJjbGllbnQuY29ubmVjdCIsImNsaWVudC5tZXRhZGF0YS5yZWFkIiwicGFja2V0LnJvdXRlIl0sImFsbG93ZWRfdGFyZ2V0cyI6WyJkYWVtb24tMSJdLCJpYXQiOjEwMDAwMDAwMDAsImV4cCI6MTAwMDAwMzYwMCwianRpIjoiY29uZm9ybWFuY2UtdG9rZW4tMSIsImFjY291bnRfaWQiOiJhY2NvdW50LTEiLCJ1c2VyX2lkIjoidXNlci0xIiwiY2xpZW50X2lkIjoiY2xpZW50LTEiLCJzZXNzaW9uX2lkIjoic2Vzc2lvbi0xIiwicHVibGljX2tleV90aHVtYnByaW50IjoidGh1bWItMSJ9.jrT5UtSBvvHbr26_lQqVWA2dC41THlz8PdpMlb0yxRo";

#[test]
fn verifies_conformance_token_issued_by_the_typescript_issuer() {
    let verifier = RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([(
            CONFORMANCE_ISSUER.to_string(),
            CONFORMANCE_SECRET.to_string(),
        )]),
        Some(1_000_001_800_000),
    );

    let identity = verifier
        .verify(RelayAuthRequest {
            token: CONFORMANCE_TOKEN,
            action: RelayAction::ClientConnect,
            target: Some("daemon-1"),
        })
        .expect("cross-issued conformance token should verify");

    assert_eq!(identity.realm_id, "realm-1");
    assert_eq!(identity.subject, "client-1");
    assert_eq!(identity.subject_kind, RelaySubjectKind::Client);
    assert_eq!(identity.token_id.as_deref(), Some("conformance-token-1"));
    assert_eq!(identity.user_id.as_deref(), Some("user-1"));
    assert_eq!(identity.public_key_thumbprint.as_deref(), Some("thumb-1"));
}

#[test]
fn conformance_token_carries_session_id_through_claim_parsing() {
    let parts = CONFORMANCE_TOKEN.split('.').collect::<Vec<_>>();
    let claims_payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("conformance claims decode");
    let claims = parse_cloud_jwt_claims(&claims_payload).expect("conformance claims parse");
    assert_eq!(claims.session_id.as_deref(), Some("session-1"));
}

#[test]
fn validate_claims_rejects_implausible_future_issue_times() {
    let claims = RelayTokenClaims {
        issuer: "issuer".to_string(),
        subject: "client-1".to_string(),
        subject_kind: RelaySubjectKind::Client,
        realm_id: "realm-1".to_string(),
        allowed_actions: vec![RelayAction::ClientConnect],
        allowed_targets: None,
        issued_at_ms: 10_000_000,
        expires_at_ms: 10_100_000,
        token_id: "token-1".to_string(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: None,
        client_id: None,
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    };
    // Issued far in the future relative to now: reject.
    assert_eq!(
        validate_claims(&claims, RelayAction::ClientConnect, None, Some(1_000)),
        Err(RelayAuthError::InvalidToken)
    );
    // Within the skew tolerance: accepted.
    assert_eq!(
        validate_claims(
            &claims,
            RelayAction::ClientConnect,
            None,
            Some(10_000_000 - RELAY_CLOCK_SKEW_TOLERANCE_MS + 1),
        ),
        Ok(())
    );
}

#[test]
fn scoped_verifier_rejects_revoked_token_ids_and_accounts() {
    let claims = RelayTokenClaims {
        issuer: "issuer".to_string(),
        subject: "client-1".to_string(),
        subject_kind: RelaySubjectKind::Client,
        realm_id: "realm-1".to_string(),
        allowed_actions: vec![RelayAction::ClientConnect],
        allowed_targets: Some(vec!["daemon-1".to_string()]),
        issued_at_ms: 10,
        expires_at_ms: 1_000,
        token_id: "token-1".to_string(),
        account_id: Some("account-1".to_string()),
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: None,
        client_id: Some("client-1".to_string()),
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    };
    let mut tokens = BTreeMap::new();
    tokens.insert("client-token".to_string(), claims.clone());
    let revocations = RelayRevocationRegistry::new();
    let verifier = ScopedTokenVerifier::new(tokens.clone(), BTreeMap::new(), Some(100))
        .with_revocations(revocations.clone());
    let request = || RelayAuthRequest {
        token: "client-token",
        action: RelayAction::ClientConnect,
        target: Some("daemon-1"),
    };

    verifier
        .verify(request())
        .expect("token verifies before revocation");

    revocations.revoke_token_id("token-1", 1_000);
    assert_eq!(
        verifier
            .verify(request())
            .expect_err("revoked jti rejected"),
        RelayAuthError::TokenRevoked
    );

    // Pruning past the token's expiry drops the entry.
    revocations.prune(2_000);
    verifier
        .verify(request())
        .expect("pruned jti revocation no longer applies");

    // Account-level revocation blocks every token for that account.
    revocations.revoke_account("account-1", 1_000);
    assert_eq!(
        verifier
            .verify(request())
            .expect_err("revoked account rejected"),
        RelayAuthError::TokenRevoked
    );
    revocations.prune(2_000);
    verifier
        .verify(request())
        .expect("pruned account revocation no longer applies");

    // Subject-level revocation blocks tokens by client/machine id, the way
    // the hosted control plane revokes paired identities.
    revocations.revoke_subject("client-1", 1_000);
    assert_eq!(
        verifier
            .verify(request())
            .expect_err("revoked client subject rejected"),
        RelayAuthError::TokenRevoked
    );
    revocations.prune(2_000);

    // A verifier without the registry is unaffected.
    let unrevoked = ScopedTokenVerifier::new(tokens, BTreeMap::new(), Some(100));
    unrevoked
        .verify(request())
        .expect("verifier without revocations still accepts the token");
}

#[test]
fn scoped_hmac_verifier_accepts_signed_hosted_issuer_tokens() {
    let claims = RelayTokenClaims {
        issuer: "arroba-cloud-test".to_string(),
        subject: "client-1".to_string(),
        subject_kind: RelaySubjectKind::Client,
        realm_id: "realm-1".to_string(),
        allowed_actions: vec![RelayAction::ClientConnect, RelayAction::ClientMetadataRead],
        allowed_targets: Some(vec!["daemon-1".to_string()]),
        issued_at_ms: 10,
        expires_at_ms: 20,
        token_id: "token-1".to_string(),
        account_id: Some("account-1".to_string()),
        organization_id: None,
        user_id: Some("user-1".to_string()),
        device_id: Some("device-1".to_string()),
        machine_id: None,
        client_id: Some("client-1".to_string()),
        session_id: None,
        public_key_thumbprint: Some("thumbprint".to_string()),
        entitlements_version: Some("entitlements-1".to_string()),
    };
    let token = encode_scoped_hmac_token(&claims, "issuer-secret").expect("token should encode");
    let verifier = RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([("arroba-cloud-test".to_string(), "issuer-secret".to_string())]),
        Some(15),
    );

    let identity = verifier
        .verify(RelayAuthRequest {
            token: &token,
            action: RelayAction::ClientConnect,
            target: Some("daemon-1"),
        })
        .expect("signed hosted issuer token should verify");

    assert_eq!(identity.realm_id, "realm-1");
    assert_eq!(identity.subject, "client-1");
    assert_eq!(identity.subject_kind, RelaySubjectKind::Client);
    assert_eq!(identity.token_id.as_deref(), Some("token-1"));
    assert_eq!(identity.user_id.as_deref(), Some("user-1"));
    assert_eq!(
        identity.public_key_thumbprint.as_deref(),
        Some("thumbprint")
    );
}

#[test]
fn legacy_scoped_token_verification_is_counted_for_deprecation() {
    let before = legacy_scoped_token_verification_count();
    let claims = RelayTokenClaims {
        issuer: "arroba-cloud-legacy".to_string(),
        subject: "client-1".to_string(),
        subject_kind: RelaySubjectKind::Client,
        realm_id: "realm-1".to_string(),
        allowed_actions: vec![RelayAction::ClientConnect],
        allowed_targets: None,
        issued_at_ms: 10,
        expires_at_ms: 20,
        token_id: "token-1".to_string(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: None,
        client_id: None,
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    };
    let token =
        encode_scoped_hmac_token(&claims, "issuer-secret").expect("legacy token should encode");
    let verifier = RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([(
            "arroba-cloud-legacy".to_string(),
            "issuer-secret".to_string(),
        )]),
        Some(15),
    );

    verifier
        .verify(RelayAuthRequest {
            token: &token,
            action: RelayAction::ClientConnect,
            target: None,
        })
        .expect("legacy arroba-scoped-v1 token should still verify");

    assert!(legacy_scoped_token_verification_count() > before);
}

#[test]
fn scoped_hmac_verifier_rejects_tampered_or_unknown_issuer_tokens() {
    let claims = RelayTokenClaims {
        issuer: "arroba-cloud-test".to_string(),
        subject: "client-1".to_string(),
        subject_kind: RelaySubjectKind::Client,
        realm_id: "realm-1".to_string(),
        allowed_actions: vec![RelayAction::ClientConnect],
        allowed_targets: None,
        issued_at_ms: 10,
        expires_at_ms: 20,
        token_id: "token-1".to_string(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: None,
        client_id: Some("client-1".to_string()),
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    };
    let token = encode_scoped_hmac_token(&claims, "issuer-secret").expect("token should encode");
    let mut parts = token.split('.').map(str::to_string).collect::<Vec<_>>();
    parts[2].push('x');
    let tampered = parts.join(".");
    let verifier = RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([("arroba-cloud-test".to_string(), "issuer-secret".to_string())]),
        Some(15),
    );
    let unknown_issuer = RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([("other-issuer".to_string(), "issuer-secret".to_string())]),
        Some(15),
    );

    assert_eq!(
        verifier
            .verify(RelayAuthRequest {
                token: &tampered,
                action: RelayAction::ClientConnect,
                target: None,
            })
            .expect_err("tampered token should fail"),
        RelayAuthError::InvalidToken
    );
    assert_eq!(
        unknown_issuer
            .verify(RelayAuthRequest {
                token: &token,
                action: RelayAction::ClientConnect,
                target: None,
            })
            .expect_err("unknown issuer should fail"),
        RelayAuthError::InvalidToken
    );
}

#[test]
fn scoped_hmac_verifier_accepts_cloud_jwt_tokens() {
    let header = serde_json::json!({
        "alg": "HS256",
        "typ": "JWT",
    });
    let claims = serde_json::json!({
        "iss": "arroba-cloud-test",
        "sub": "client-1",
        "subject_kind": "client",
        "realm_id": "realm-1",
        "allowed_actions": ["client.connect", "client.metadata.read", "packet.route"],
        "allowed_targets": ["daemon-1"],
        "iat": 10,
        "exp": 20,
        "jti": "token-1",
        "account_id": "account-1",
        "client_id": "client-1",
        "public_key_thumbprint": "thumbprint",
    });
    let header_b64 =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("jwt header should serialize"));
    let claims_b64 =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("jwt claims should serialize"));
    let signing_input = format!("{header_b64}.{claims_b64}");
    let signature =
        URL_SAFE_NO_PAD.encode(sign_hmac(b"issuer-secret", signing_input.as_bytes()).unwrap());
    let token = format!("{signing_input}.{signature}");
    let verifier = RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([("arroba-cloud-test".to_string(), "issuer-secret".to_string())]),
        Some(15_000),
    );

    let identity = verifier
        .verify(RelayAuthRequest {
            token: &token,
            action: RelayAction::ClientConnect,
            target: Some("daemon-1"),
        })
        .expect("jwt cloud token should verify");

    assert_eq!(identity.realm_id, "realm-1");
    assert_eq!(identity.subject, "client-1");
    assert_eq!(identity.subject_kind, RelaySubjectKind::Client);
    assert_eq!(identity.token_id.as_deref(), Some("token-1"));
    assert_eq!(
        identity.public_key_thumbprint.as_deref(),
        Some("thumbprint")
    );
}

#[test]
fn scoped_hmac_verifier_rejects_cloud_jwt_tokens_with_unknown_actions() {
    let header = serde_json::json!({
        "alg": "HS256",
        "typ": "JWT",
    });
    let claims = serde_json::json!({
        "iss": "arroba-cloud-test",
        "sub": "client-1",
        "subject_kind": "client",
        "realm_id": "realm-1",
        "allowed_actions": ["client.connect", "workflow.run"],
        "iat": 10,
        "exp": 20,
        "jti": "token-1",
    });
    let header_b64 =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("jwt header should serialize"));
    let claims_b64 =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("jwt claims should serialize"));
    let signing_input = format!("{header_b64}.{claims_b64}");
    let signature =
        URL_SAFE_NO_PAD.encode(sign_hmac(b"issuer-secret", signing_input.as_bytes()).unwrap());
    let token = format!("{signing_input}.{signature}");
    let verifier = RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([("arroba-cloud-test".to_string(), "issuer-secret".to_string())]),
        Some(15_000),
    );

    assert_eq!(
        verifier
            .verify(RelayAuthRequest {
                token: &token,
                action: RelayAction::ClientConnect,
                target: None,
            })
            .expect_err("unknown cloud action should fail"),
        RelayAuthError::InvalidToken
    );
}
