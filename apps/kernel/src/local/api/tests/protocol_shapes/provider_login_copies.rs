//! MP-08/MP-10/MP-11 credfix: sign-out of a login that other machines hold removes only the local copy.
use super::*;

#[test]
fn credfix_logout_shapes_are_versioned_hashed_and_backward_compatible() {
    use sha2::{Digest, Sha256};
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
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
        "b1f609ad8ca7a4ff9ea980d132c793b6428b960a2d040041a8f61b23f6f8f4db"
    );
}
