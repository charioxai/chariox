//! Local 1069: a grant says what it follows (`KernelAccessStatus.follows`, an empty subscription), and
//! `SubscribeEvents.like` takes over the targets an earlier live grant of the same owner follows.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn event_follow_1069_snapshot_and_hash() {
    let bytes = include_bytes!("event-follow-1069.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "e4975a9d3eca8e712673f6ebd5d178ca73a4dbe1112ffaad9f44e08add69f54c"
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
