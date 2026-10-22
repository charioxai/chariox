//! Room terminal request and result snapshot and hash.
use super::*;

fn assert_round_trip(wire: serde_json::Value, expected_sha256: &str) {
    use sha2::{Digest, Sha256};
    let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    let encoded = serde_json::to_value(request).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        expected_sha256
    );
}

#[test]
fn room_terminal_protocol_requests_snapshot_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_round_trip(
        serde_json::json!({"RoomTerminal": {
            "session_id": "room-a",
            "operation": {"operation": "open", "cols": 100, "rows": 30}
        }}),
        "fda70d2f8b97d306b0f93f048b302f9316b8b8fda89be2be913667c525f6ece7",
    );
    assert_round_trip(
        serde_json::json!({"RoomTerminal": {
            "session_id": "room-a",
            "acting_agent": "agent-1",
            "operation": {"operation": "input", "terminal_id": "term-1", "data_base64": "bHMNCg=="}
        }}),
        "f63a2067883cd391ac1a88656965d7c2500a2204300c45dabe785c071672cce7",
    );
    assert_round_trip(
        serde_json::json!({"RoomTerminal": {
            "session_id": "room-a",
            "operation": {
                "operation": "wait", "terminal_id": "term-1", "after_offset": 42,
                "watches": [{"kind": "finished"}, {"kind": "output_contains", "text": "ready"}, {"kind": "waiting_for_input"}],
                "timeout_ms": 240000
            }
        }}),
        "b04036b0df3810f15b001625952526db578c86a3486365e901a3567e58f765a0",
    );
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({"RoomTerminal": {
            "session_id": "room-a",
            "operation": {"operation": "run_anything"}
        }}))
        .is_err()
    );
}

#[test]
fn room_terminal_protocol_results_snapshot_and_hash() {
    use crate::local::RoomTerminalResult;
    use sha2::{Digest, Sha256};
    let wire = serde_json::json!({"RoomTerminal": {"result": {"result": "input", "input": {
        "accepted": false,
        "driver": {"state": "busy", "driven_by": {"kind": "agent", "agent_id": "agent-1"}},
        "refusal": "busy (driven by agent-1)"
    }}}});
    let response: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
    assert!(matches!(
        &response,
        LocalDaemonResponse::RoomTerminal {
            result: RoomTerminalResult::Input { .. }
        }
    ));
    let encoded = serde_json::to_value(response).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "909020e37bf1bb285c875761906626455d18750481536875ae14b21c0b8a8542"
    );
}
