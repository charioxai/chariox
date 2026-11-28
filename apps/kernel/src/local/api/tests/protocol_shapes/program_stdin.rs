//! A program opened in a terminal reads `stdin`: its bytes inline or a file on the kernel.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn program_stdin_open_snapshot_and_hash() {
    let bytes = include_bytes!("program-stdin-1014.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "f3f7fe00bc3eca90640eaea1fd669c134096f3fc5a351e3cb282ff1ca2f981c9"
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
