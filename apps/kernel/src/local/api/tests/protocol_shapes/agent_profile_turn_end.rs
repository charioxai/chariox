use super::*;
use sha2::{Digest, Sha256};

#[test]
fn agent_profile_turn_end_snapshot_and_hash() {
    let bytes = include_bytes!("agent-profile-turn-end.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "1461aafc7d5f367a318c30ad7048fd73f5fc7566cae0ef48c1cf0be7c9a9a45c"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *wire);
    }
    let wire = &fixture["response"];
    let response: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), *wire);
}
