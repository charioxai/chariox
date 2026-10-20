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
        "d162ebb07acce6f378880057bdf0cb5ab0b3cff3e794804aa2688f8ca4751337"
    );
}
