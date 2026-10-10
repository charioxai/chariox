use super::*;
use crate::local::{
    IssueCloudRelayClientTokenRequest, JoinTerminalPairingLinkRequest, PairingInviteIntent,
    PairingJoinRecord, TerminalRecord, TerminalType,
};

#[test]
fn relay_status_control_capabilities_are_versioned_and_hashed() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 484);
    let legacy = serde_json::json!({
        "configured": false, "connected": false, "relay_url": null,
        "relay_token_configured": false, "daemon_id": "kernel-1",
        "daemon_alias": null, "machine_id": "machine-1", "machine_alias": null
    });
    let mut status: crate::local::RelayStatus = serde_json::from_value(legacy).unwrap();
    assert!(
        status.capabilities.is_empty(),
        "old kernels must not gain implied support"
    );
    status.capabilities = crate::local::RUNTIME_CONTROL_CAPABILITIES
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    let response = serde_json::to_value(LocalDaemonResponse::RelayStatus { status }).unwrap();
    assert_eq!(
        response["RelayStatus"]["status"]["capabilities"],
        serde_json::json!([
            "disposable_worker_control_v1",
            "managed_environment_keep_running_v1",
            "terminal_relay_authorization_renewal_v1"
        ])
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&response).unwrap().as_bytes())
        ),
        "e62f3ae9cec132c2178aa7b5c738669368b12398eb06ac67439a9f1721c1c13b"
    );
}

#[test]
fn key_bound_cli_relay_requests_and_join_response_have_exact_protocol_shapes() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 484);

    let token_request =
        LocalDaemonRequest::IssueCloudRelayClientToken(IssueCloudRelayClientTokenRequest {
            target_daemon_alias: "home-kernel".to_string(),
            client_id: "terminal-1".to_string(),
            session_id: Some("session-1".to_string()),
            public_key_thumbprint: Some("cli-thumbprint".to_string()),
        });
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

    let join_request =
        LocalDaemonRequest::JoinTerminalPairingLink(JoinTerminalPairingLinkRequest {
            pairing_link: "chariox-terminal-pair-v1.fixture".to_string(),
            terminal_id: Some("terminal-1".to_string()),
            terminal_type: Some(TerminalType::Cli),
            alias: None,
            public_key_thumbprint: Some("cli-thumbprint".to_string()),
        });
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

#[test]
fn relay_status_native_process_identity_is_versioned_and_hashed() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 484);
    let legacy = serde_json::json!({
        "configured": false, "connected": false, "relay_url": null,
        "relay_token_configured": false, "daemon_id": "kernel-1",
        "daemon_alias": null, "machine_id": "machine-1", "machine_alias": null
    });
    let mut status: crate::local::RelayStatus = serde_json::from_value(legacy).unwrap();
    assert!(status.runtime_process_identity.is_none());
    status.runtime_process_identity = Some(crate::local::KernelRuntimeProcessIdentity {
        pid: 4321,
        linux_boot_id: "b4a8b0e7-0f5b-4fd8-bcd9-ccc1e8b3c5ac".to_string(),
        start_time_ticks: "7712345".to_string(),
    });
    let response = serde_json::to_value(LocalDaemonResponse::RelayStatus { status }).unwrap();
    assert_eq!(
        response["RelayStatus"]["status"]["runtime_process_identity"]["pid"],
        4321
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&response).unwrap().as_bytes())
        ),
        "c3dfd43214945bfdc036638d58c9724d9d572cb00267c90aef7bb975241e8cef"
    );
}
