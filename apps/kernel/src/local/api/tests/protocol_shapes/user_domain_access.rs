//! MP-08/MP-11: allocated 507/115 grant and reachability shape guard.
use super::*;
use crate::local::*;
use sha2::{Digest, Sha256};

#[test]
fn mdaccess_protocol_462_grant_shapes_and_hash() {
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
    let grant = UserDomainGrant {
        agent_id: "a".into(),
        session_id: "s".into(),
        kernel_id: "home".into(),
        resources: vec![
            UserDomainResource::BrowserTab { tab_id: "t".into() },
            UserDomainResource::AppView {
                view_id: "v".into(),
            },
            UserDomainResource::Capture {
                capture_id: "c".into(),
            },
        ],
        since_ms: 1,
        focused: false,
        idle_since_ms: Some(2),
        idle_timeout_seconds: 1800,
        expiry_rule: "active turn or pending wake; then idle window".into(),
        prompt_id: Some("p".into()),
        delegated_by_agent_id: None,
        expires_at_ms: Some(28_800_001),
    };
    let delegated = UserDomainGrant {
        agent_id: "child".into(),
        prompt_id: None,
        delegated_by_agent_id: Some("a".into()),
        resources: vec![UserDomainResource::BrowserTab { tab_id: "t".into() }],
        ..grant.clone()
    };
    values.push(serde_json::to_value(LocalDaemonResponse::KernelBrowser { result: serde_json::json!({"event":"user_domain_grants_changed","cursor":4,"room_computer":[{"agent_id":"a","session_id":"s","allowed":false}],"grants":[grant, delegated],"notice":UserDomainNotice {agent_id:"a".into(),resource:UserDomainResource::BrowserTab {tab_id:"t".into()},at_ms:3}}) }).unwrap());
    values.push(
        serde_json::to_value(LocalDaemonResponse::UserAppViewsListed {
            views: vec![UserAppView {
                view_id: "v".into(),
                installation_id: "app".into(),
                generation: "7".into(),
                origin: "https://app.a.invalid".into(),
                browser: None,
                access: Some(UserDomainWindowAccess {
                    kernel_id: "home".into(),
                    kernel_name: "home".into(),
                    focused_agent_kernel_id: Some("worker".into()),
                    reachable_by_focused_agent: false,
                }),
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
        "ed257e14562fd553bf4f6387367098d4d30f356258156225ce56a01a17c7a2c5"
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
        "f17959b53e04da182601333225ee3038d65944c9b51d14032aef8524c89a2e8d"
    );
}
