use super::*;
use sha2::{Digest, Sha256};

#[test]
fn local_daemon_protocol_provider_account_credential_shape_is_versioned() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );

    let request = LocalDaemonRequest::SetProviderAccountCredential(
        crate::local::SetProviderAccountCredentialRequest {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            provider: "claude".to_string(),
            account_profile: "work".to_string(),
            value: "setup-token-secret".to_string(),
            run: true,
            overwrite: true,
        },
    );
    let legacy: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "SetProviderAccountCredential": {"provider":"claude", "account_profile":"work", "value":"synthetic", "overwrite":false}
    })).unwrap();
    assert!(
        matches!(legacy, LocalDaemonRequest::SetProviderAccountCredential(ref request) if !request.run)
    );
    let response = LocalDaemonResponse::ProviderAccountCredentialStored {
        provider: "claude".to_string(),
        account_profile: "work".to_string(),
        credential_id: "provider-account-claude-handle".to_string(),
        replaced: true,
    };
    let snapshot = serde_json::json!([request, response]);
    assert_eq!(
        snapshot.pointer("/0/SetProviderAccountCredential/account_profile"),
        Some(&serde_json::json!("work"))
    );
    assert_eq!(
        snapshot.pointer("/0/SetProviderAccountCredential/overwrite"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        snapshot.pointer("/1/ProviderAccountCredentialStored/credential_id"),
        Some(&serde_json::json!("provider-account-claude-handle"))
    );

    assert_eq!(
        snapshot.pointer("/0/SetProviderAccountCredential/run"),
        Some(&serde_json::json!(true))
    );
    let encoded = serde_json::to_string(&snapshot).expect("snapshot should encode");
    let hash = Sha256::digest(encoded.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "e525fa0b75a08d3ce4267c0fe016d37a5ae1fea0d8734368bb3887bc833cf8bc"
    );
}

#[test]
fn account_copy_public_and_encrypted_peer_requests_are_admitted() {
    let source = serde_json::json!({"CopyProviderAccountToKernel": {
        "provider":"claude", "account_profile":"work-exact", "operation_id":"op-1",
        "target_kernel_id":"target", "target_key_thumbprint":"a".repeat(64), "replace":false
    }});
    let parsed = serde_json::from_value::<LocalDaemonRequest>(source.clone());
    assert!(parsed.is_ok(), "MP-08/MP-09/MP-10/MP-11 source request must be supported");
    assert_eq!(serde_json::to_value(parsed.unwrap()).unwrap(), source);
    let private = crate::transport::relay_crypto::generate_private_key_base64();
    let public = crate::transport::relay_crypto::public_key_from_private_key_base64(&private).unwrap();
    let peer = serde_json::json!({"kind":"install_provider_account_copy", "binding": {
        "operation_id":"op-1", "provider":"claude", "source_profile_id":"work-exact",
        "source_kernel_id":"source", "source_key_thumbprint":"b".repeat(64),
        "target_kernel_id":"target", "target_key_thumbprint":"a".repeat(64), "replace":false
    }, "credential":"synthetic-copy-canary"});
    let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(&private, &public, &serde_json::to_vec(&peer).unwrap()).unwrap();
    let plaintext = crate::transport::relay_crypto::decrypt_payload_for_private_key(&private, &encrypted).unwrap();
    let request = serde_json::from_slice::<crate::transport::relay_peer::RelayPeerRequest>(&plaintext.plaintext);
    assert!(request.is_ok(), "encrypted account install must be supported");
    assert!(!format!("{:?}", request.unwrap()).contains("synthetic-copy-canary"));
}
