//! MP-08/MP-11: allocated449/92 grant and reachability shape guard.
use super::*;
use crate::local::*;
use sha2::{Digest, Sha256};

#[test]
fn mdaccess_protocol_449_grant_shapes_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 449);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        92
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
            agent_id: "a".into(),
        },
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
            UserDomainResource::Note {
                note_id: "n".into(),
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
    };
    values.push(serde_json::to_value(LocalDaemonResponse::KernelBrowser { result: serde_json::json!({"event":"user_domain_grants_changed","cursor":4,"room_computer":[{"agent_id":"a","session_id":"s","allowed":false}],"grants":[grant],"notice":UserDomainNotice {agent_id:"a".into(),resource:UserDomainResource::BrowserTab {tab_id:"t".into()},at_ms:3}}) }).unwrap());
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
        serde_json::from_str(include_str!("user-domain-access-449.json")).unwrap();
    assert_eq!(serde_json::Value::Array(values), expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&expected).unwrap())
        ),
        "1037eeb4924dbc21b31b8289e12dc6881d805327c3b50086018d622527857e4b"
    );
    assert!(serde_json::from_value::<KernelBrowserRequest>(
        serde_json::json!({"command":{"op":"revoke_grants","agent_id":null,"owner":"forged"}})
    )
    .is_err());
}
