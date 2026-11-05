use super::*;

// MP-08 / MP-09 / MP-10 / MP-11 VAUDIT S2 (943/392): the `env` credential
// and `file` uses and injection kinds of credentials captured by the kernel.
#[test]
fn mp08_mp11_env_credential_943_shape_and_hash() {
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 943);
    }
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 392);
    }
    let wire = r#"{"id":"cap-0123456789abcdef01234567","description":"example key","source":{"type":"vault","key":"cap-0123456789abcdef01234567"},"allowed_hosts":["api.example.com"],"allowed_uses":["env"],"injection":{"kind":"env"},"metadata":{"created_by_kind":"vault_capture","created_by_id":"agent-1","session_id":"session-1","vault_key":"cap-0123456789abcdef01234567"}}"#;
    let credential: crate::config::UserCredentialConfig = serde_json::from_str(wire).unwrap();
    assert_eq!(
        credential.allowed_uses,
        [crate::config::UserCredentialUse::Env]
    );
    assert_eq!(
        credential.injection,
        crate::config::UserCredentialInjectionConfig::Env
    );
    let bytes = serde_json::to_vec(&credential).unwrap();
    assert_eq!(bytes, wire.as_bytes());
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        "b208e07668ede1a18076ef3b281274d225a7ab208c8912ae713d348ecd65d2f7"
    );
    let file = r#"{"id":"cap-file","source":{"type":"vault","key":"cap-file"},"allowed_uses":["env","file"],"injection":{"kind":"file"}}"#;
    let credential: crate::config::UserCredentialConfig = serde_json::from_str(file).unwrap();
    assert_eq!(
        credential.injection,
        crate::config::UserCredentialInjectionConfig::File
    );
    assert_eq!(serde_json::to_vec(&credential).unwrap(), file.as_bytes());
}
