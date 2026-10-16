//! MP-08/MP-10/MP-11 credfix: sign-out of a login that other machines hold removes only the local copy.
use super::*;

#[test]
fn credfix_logout_shapes_are_versioned_hashed_and_backward_compatible() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 541);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        149
    );
    let snapshot = serde_json::json!({
        "local_protocol": LOCAL_DAEMON_PROTOCOL_VERSION,
        "peer_protocol": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        "requests": [
            {"LogoutProvider": {"provider": "codex", "account_profile": "work"}},
            {"LogoutProvider": {"provider": "codex", "account_profile": "work", "end_everywhere": true}}
        ],
        "responses": [
            {"ProviderLoggedOut": {"provider": "codex", "account_profile": "work"}},
            {"ProviderLoggedOut": {"provider": "codex", "account_profile": "work", "local_only": true}}
        ]
    });
    for request in snapshot["requests"].as_array().unwrap() {
        let typed: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *request);
    }
    for response in snapshot["responses"].as_array().unwrap() {
        let typed: LocalDaemonResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), *response);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "bc88f3f902709065c58d5a5c01fdd14cb479b3330997d298aaa830e388e48679"
    );
}
