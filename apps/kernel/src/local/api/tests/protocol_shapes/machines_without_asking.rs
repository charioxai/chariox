//! Local 1063: the owner's limits for machines agents acquire without asking are user config keys.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn machines_without_asking_1063_snapshot_and_hash() {
    let bytes = include_bytes!("machines-without-asking-1063.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "a53f127a0b2aa80d22a7b60991360469db4a035d9648994ed271a114eb9c933c"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    for wire in fixture["requests"].as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
    for wire in fixture["agents_configs"].as_array().unwrap() {
        let parsed: crate::config::UserAgentsConfig = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *wire);
    }
}
