//! Local 1015: the owner's limits for machines agents acquire without asking are user config keys.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn machines_without_asking_1015_snapshot_and_hash() {
    let bytes = include_bytes!("machines-without-asking-1015.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "2b546dc0db0b366fc7328fab10dcd0034c2417866dacce0dd6c2215cbcee0e4e"
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
