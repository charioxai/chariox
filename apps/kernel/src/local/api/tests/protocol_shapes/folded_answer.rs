//! A folded delivery keeps the one native answer on its own turn.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn folded_answer_receipt_snapshot_and_hash() {
    let bytes = include_bytes!("folded-answer-1013.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "97608fe112efe8310638820ddb8161a4397ca2dc3053106c97edca260ba36016"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["protocol_version"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["relay_peer_protocol_version"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let outcome: crate::provider::TurnOutcome =
        serde_json::from_value(fixture["outcome"].clone()).unwrap();
    assert!(!outcome.is_error());
    assert_eq!(serde_json::to_value(outcome).unwrap(), fixture["outcome"]);
}
