//! A Room-wide terminal list names the machines it could not ask; a list every machine answered has no such field.
use super::*;
use crate::local::RoomTerminalResult;
use sha2::{Digest, Sha256};

#[test]
fn room_terminal_list_unavailable_snapshot_and_hash() {
    let bytes = include_bytes!("room-terminal-list-unavailable-1016.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "56eea581c1c3fa03eab8be311b8e8f1d71e347fd320611ddd7fc5c6f7163f8be"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["relayPeer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for name in ["partial_list", "full_list"] {
        let wire = fixture[name].clone();
        let parsed: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), wire, "{name}");
    }
    let LocalDaemonResponse::RoomTerminal {
        result: RoomTerminalResult::Listed { unavailable, .. },
    } = serde_json::from_value(fixture["partial_list"].clone()).unwrap()
    else {
        panic!("the partial list is a terminal list");
    };
    assert_eq!(unavailable.len(), 1);
    assert_eq!(unavailable[0].host_agent, "worker-agent");
}
