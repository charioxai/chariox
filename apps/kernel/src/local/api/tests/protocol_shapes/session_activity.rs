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
        serde_json::json!({"local":1014,"peer": 451,"schema":14,"session":{"last_activity_at_ms":42}})
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "a25d8337484b14e624bca07a9724cb01b3239eb03ad848ed950a88a3d5569352"
    );
}

#[test]
fn provider_failure_and_deadline_status_are_versioned() {
    use crate::provider::TurnOutcomeKind;
    use crate::runtime::projection::AgentRuntimeStatus;
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        serde_json::json!(AgentRuntimeStatus::EndedDeadline),
        "ended_deadline"
    );
    assert_eq!(
        serde_json::json!(TurnOutcomeKind::ModelRefused),
        "model_refused"
    );
    assert_eq!(
        serde_json::json!(TurnOutcomeKind::StreamEnded),
        "stream_ended"
    );
}
