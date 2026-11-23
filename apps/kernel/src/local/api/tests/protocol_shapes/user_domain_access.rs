//! MP-08/MP-11: no per-agent Room Computer switch and no user-domain grants.
use super::*;
use crate::local::*;
use sha2::{Digest, Sha256};

#[test]
fn the_room_computer_switches_are_gone_and_a_leased_desktop_grant_answers_plainly() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    // The owner's agents need no per-agent switch: none of these commands exists any more.
    for command in [
        serde_json::json!({"op":"list_grants"}),
        serde_json::json!({"op":"subscribe_grants","after":3,"wait_ms":25000}),
        serde_json::json!({"op":"revoke_grants","agent_id":null}),
        serde_json::json!({"op":"grant_room_computer","agent_id":null}),
    ] {
        assert!(
            serde_json::from_value::<KernelBrowserRequest>(serde_json::json!({"command":command}))
                .is_err(),
            "{command} must be refused"
        );
    }
    let values = vec![
        serde_json::to_value(LocalDaemonResponse::KernelBrowser {
            result: serde_json::json!({"agent_id":"a","leased_computer":true}),
        })
        .unwrap(),
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
    ];
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("user-domain-access-507.json")).unwrap();
    assert_eq!(serde_json::Value::Array(values), expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&expected).unwrap())
        ),
        "f92b3fbefa608486667d580491c356c1cbe882a44cbf87747b902d75bf3dfd0c"
    );
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
        "023daaf087016143be6c3a6bcc34e1ab67c38a39efd7997cf24ec392750fdcb8"
    );
}
