use super::*;
use sha2::{Digest, Sha256};

#[test]
fn semantic_session_activity_protocol_snapshot_and_hash() {
    let session = crate::session::RuntimeSession::new(
        "session",
        crate::session::ProjectId::test(),
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    let mut wire = serde_json::to_value(session).unwrap();
    wire["last_activity_at_ms"] = serde_json::json!(42);
    let decoded: crate::session::RuntimeSession = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded.last_activity_at_ms(), 42);
    let snapshot = serde_json::json!({"local": LOCAL_DAEMON_PROTOCOL_VERSION,
        "peer": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        "schema": 14, "session": {"last_activity_at_ms": decoded.last_activity_at_ms()}});
    assert_eq!(
        snapshot,
        serde_json::json!({"local":1084,"peer": 449,"schema":14,"session":{"last_activity_at_ms":42}})
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "2a324cc0735dd7264b3a8f5b58545f8193febe9945a2e791aadda2f0e6709fcf"
    );
}
