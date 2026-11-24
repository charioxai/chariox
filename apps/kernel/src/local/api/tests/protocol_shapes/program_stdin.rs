//! A program opened in a terminal reads `stdin`: its bytes inline or a file on the kernel.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn program_stdin_open_snapshot_and_hash() {
    let bytes = include_bytes!("program-stdin-1014.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "b33218551c231b7a30519e5f7de35685a945beb20808e4c32e4fdc2f57e96a58"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["protocol_version"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["relay_peer_protocol_version"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for request in ["inline", "file"] {
        let wire = fixture[request].clone();
        let parsed: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
    }
}
