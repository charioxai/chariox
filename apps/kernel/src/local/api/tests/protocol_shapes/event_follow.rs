//! Local 1012: a grant says what it follows (`KernelAccessStatus.follows`, an empty subscription), and
//! `SubscribeEvents.like` takes over the targets an earlier live grant of the same owner follows.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn event_follow_1012_snapshot_and_hash() {
    let bytes = include_bytes!("event-follow-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "a327889dee651e60cab15ecdccd2625168e9742f84b8a6062a1d4cf51da75a8f"
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
}

#[test]
fn a_like_only_subscribe_is_the_documented_takeover() {
    // PROTOCOL.md: `SubscribeEvents { like }` is sent without `targets` or `sources`.
    let request: LocalDaemonRequest = serde_json::from_value(
        serde_json::json!({"SubscribeEvents": {"like": "access-0123456789abcdef"}}),
    )
    .unwrap();
    let LocalDaemonRequest::SubscribeEvents(subscribe) = request else {
        panic!("expected SubscribeEvents")
    };
    assert_eq!(subscribe.like.as_deref(), Some("access-0123456789abcdef"));
    assert!(subscribe.targets.is_empty() && subscribe.sources.is_empty());
}
