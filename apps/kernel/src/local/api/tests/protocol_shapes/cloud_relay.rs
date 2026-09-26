use super::*;
use crate::local::{
    IssueCloudRelayClientTokenRequest, JoinTerminalPairingLinkRequest, PairingInviteIntent,
    PairingJoinRecord, TerminalRecord, TerminalType,
};

#[test]
fn key_bound_cli_relay_requests_and_join_response_have_exact_protocol_shapes() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 349);

    let token_request = LocalDaemonRequest::IssueCloudRelayClientToken(
        IssueCloudRelayClientTokenRequest {
            target_daemon_alias: "home-kernel".to_string(),
            client_id: "terminal-1".to_string(),
            session_id: Some("session-1".to_string()),
            public_key_thumbprint: Some("cli-thumbprint".to_string()),
        },
    );
    assert_eq!(
        serde_json::to_value(token_request).expect("client token request shape"),
        serde_json::json!({
            "IssueCloudRelayClientToken": {
                "target_daemon_alias": "home-kernel",
                "client_id": "terminal-1",
                "session_id": "session-1",
                "public_key_thumbprint": "cli-thumbprint"
            }
        })
    );

    let join_request = LocalDaemonRequest::JoinTerminalPairingLink(
        JoinTerminalPairingLinkRequest {
            pairing_link: "chariox-terminal-pair-v1.fixture".to_string(),
            terminal_id: Some("terminal-1".to_string()),
            terminal_type: Some(TerminalType::Cli),
            alias: None,
            public_key_thumbprint: Some("cli-thumbprint".to_string()),
        },
    );
    assert_eq!(
        serde_json::to_value(join_request).expect("terminal join request shape"),
        serde_json::json!({
            "JoinTerminalPairingLink": {
                "pairing_link": "chariox-terminal-pair-v1.fixture",
                "terminal_id": "terminal-1",
                "terminal_type": "cli",
                "alias": null,
                "public_key_thumbprint": "cli-thumbprint"
            }
        })
    );

    let joined = LocalDaemonResponse::TerminalPairingLinkJoined {
        terminal: TerminalRecord {
            terminal_id: "terminal-1".to_string(),
            terminal_type: TerminalType::Cli,
            alias: None,
            paired_at_ms: 42,
            revoked: false,
        },
        pairing: PairingJoinRecord {
            intent: PairingInviteIntent::Client,
            subject_id: "terminal-1".to_string(),
            relay_url: "wss://relay.example".to_string(),
            target_daemon_id: "home-kernel".to_string(),
            alias: None,
            public_key_thumbprint: "cli-thumbprint".to_string(),
            paired_at_ms: 42,
        },
        relay_token: Some("fresh-bound-token".to_string()),
    };
    assert_eq!(
        serde_json::to_value(joined).expect("bound join response shape"),
        serde_json::json!({
            "TerminalPairingLinkJoined": {
                "terminal": {
                    "terminal_id": "terminal-1",
                    "terminal_type": "cli",
                    "alias": null,
                    "paired_at_ms": 42,
                    "revoked": false
                },
                "pairing": {
                    "intent": "client",
                    "subject_id": "terminal-1",
                    "relay_url": "wss://relay.example",
                    "target_daemon_id": "home-kernel",
                    "alias": null,
                    "public_key_thumbprint": "cli-thumbprint",
                    "paired_at_ms": 42
                },
                "relay_token": "fresh-bound-token"
            }
        })
    );
}

#[test]
fn legacy_terminal_join_requests_and_responses_remain_unbound() {
    let request: JoinTerminalPairingLinkRequest = serde_json::from_value(serde_json::json!({
        "pairing_link": "chariox-terminal-pair-v1.fixture"
    }))
    .expect("legacy join request should deserialize");
    assert_eq!(request.public_key_thumbprint, None);

    let response = LocalDaemonResponse::TerminalPairingLinkJoined {
        terminal: TerminalRecord {
            terminal_id: "terminal-1".to_string(),
            terminal_type: TerminalType::Cli,
            alias: None,
            paired_at_ms: 42,
            revoked: false,
        },
        pairing: PairingJoinRecord {
            intent: PairingInviteIntent::Client,
            subject_id: "terminal-1".to_string(),
            relay_url: "ws://relay.example".to_string(),
            target_daemon_id: "home-kernel".to_string(),
            alias: None,
            public_key_thumbprint: "legacy-kernel-thumbprint".to_string(),
            paired_at_ms: 42,
        },
        relay_token: None,
    };
    let serialized = serde_json::to_value(response).expect("legacy response shape");
    assert!(serialized
        .pointer("/TerminalPairingLinkJoined/relay_token")
        .is_none());
}
