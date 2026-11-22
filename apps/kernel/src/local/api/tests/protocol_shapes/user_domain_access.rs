//! MP-08/MP-11: Room Computer switch shapes (no user-domain grants).
use super::*;
use crate::local::*;
use sha2::{Digest, Sha256};

#[test]
fn room_computer_switch_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let mut values = Vec::new();
    for command in [
        KernelBrowserCommand::ListGrants,
        KernelBrowserCommand::SubscribeGrants {
            after: 3,
            wait_ms: 25000,
        },
        KernelBrowserCommand::RevokeGrants {
            agent_id: Some("a".into()),
        },
        KernelBrowserCommand::RevokeGrants { agent_id: None },
        KernelBrowserCommand::GrantRoomComputer {
            agent_id: Some("a".into()),
        },
        KernelBrowserCommand::GrantRoomComputer { agent_id: None },
    ] {
        let request = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command });
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<LocalDaemonRequest>(value.clone()).unwrap(),
            request
        );
        values.push(value);
    }
    values.push(serde_json::to_value(LocalDaemonResponse::KernelBrowser { result: serde_json::json!({"event":"user_domain_grants_changed","cursor":4,"room_computer":[{"agent_id":"a","session_id":"s","allowed":false}]}) }).unwrap());
    values.push(
        serde_json::to_value(LocalDaemonResponse::UserAppViewsListed {
            views: vec![UserAppView {
                view_id: "v".into(),
                installation_id: "app".into(),
                generation: "7".into(),
                origin: "https://app.a.invalid".into(),
                browser: None,
            }],
        })
        .unwrap(),
    );
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("user-domain-access-507.json")).unwrap();
    assert_eq!(serde_json::Value::Array(values), expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&expected).unwrap())
        ),
        "d904ca32d5953be83f725a0a5ff67c0dc53069a539f5f853937a72c4cab13912"
    );
    assert!(serde_json::from_value::<KernelBrowserRequest>(
        serde_json::json!({"command":{"op":"revoke_grants","agent_id":null,"owner":"forged"}})
    )
    .is_err());
    // Bulk restore needs an explicit null; an omitted agent must not restore everyone.
    assert!(serde_json::from_value::<KernelBrowserRequest>(
        serde_json::json!({"command":{"op":"grant_room_computer"}})
    )
    .is_err());
}

// MP-08/MP-11: keep the typed desktop grant separate from Room browser packets.
#[test]
fn mp11_leased_computer_protocol_507_115_snapshot() {
    use crate::transport::relay_peer::{
        RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
    };
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("leased-computer-507.json")).unwrap();
    assert_eq!(expected["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(expected["peer_protocol"], RELAY_PEER_PROTOCOL_VERSION);
    for value in expected["local"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    for value in expected["peer"].as_array().unwrap() {
        let request: RelayPeerRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    for value in expected["responses"].as_array().unwrap() {
        let response: RelayPeerResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *value);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&expected).unwrap())
        ),
        "6fb4eb1e8c30e63a9ec9b57426bde4ae63e59209df363796f5830de6ec67ad50"
    );
}
