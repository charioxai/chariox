//! Local 1012: the person's Inbox (docs/NOTIFICATION_CENTER_PLAN.md section 4): its cards, read, snooze and mute.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn person_inbox_1012_snapshot_and_hash() {
    let bytes = include_bytes!("person-inbox-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "ca0beb85d693639ee41d12b4680d94dd102b3b4253369414c19c627eecc1b11d"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["responses"].as_array().unwrap() {
        let parsed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    // A card's state is the kernel's: a request names cards and a change, nothing else.
    for forged in [
        json!({"SetInboxCardState":{"card_ids":["n"],"change":{"read":{"read":true}},"owner":"someone-else"}}),
        json!({"SetInboxCardState":{"card_ids":["n"],"change":{"mute":{}}}}),
        json!({"SetInboxMute":{"origin":{"session_id":"room-1"},"muted":true}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
