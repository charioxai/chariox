use super::*;
use sha2::{Digest, Sha256};

#[test]
fn local_daemon_protocol_provider_account_credential_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 478);

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
