use super::*;
use sha2::{Digest, Sha256};

#[test]
fn semantic_session_activity_protocol_snapshot_and_hash() {
    let session = crate::session::RuntimeSession::new(
        "session",
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
        serde_json::json!({"local":811,"peer":361,"schema":14,"session":{"last_activity_at_ms":42}})
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "495358b34c73ccc69bc7f87bf9b8d863cc860bcdbea289ee4c3df64ff2e45403"
    );
}
