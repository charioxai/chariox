//! MP-08 / MP-09 / MP-10 / MP-11: owner gate control and entry acceptance wire.
use super::*;
use sha2::{Digest, Sha256};
#[test]
fn workflow_gate_entry_and_owned_state_protocol_snapshot() {
    let snapshot: serde_json::Value =
        serde_json::from_str(include_str!("workflow-gate-1002.json")).unwrap();
    assert_eq!(snapshot["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        snapshot["peer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for wire in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *wire);
    }
    for wire in snapshot["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *wire);
    }
    for wire in snapshot["node_run_waiting_reasons"].as_array().unwrap() {
        let reason: crate::session::WorkflowNodeWaitingReason =
            serde_json::from_value(wire.clone()).unwrap();
        let mut node_run = crate::session::WorkflowNodeRun::new(
            "node-run",
            "node",
            "agent",
            0,
            crate::session::WorkflowNodeRunStatus::Ready,
        );
        node_run.set_waiting_reason(reason);
        assert_eq!(
            serde_json::to_value(node_run).unwrap()["waiting_reason"],
            *wire
        );
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "fcfad22166dbd5dc4e2cd83812cdb75edd2c13569fb38eef873fb25fedf39b37"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({"WorkflowGate":{"session_id":"room","workflow_ref":"advisory","action":{"kind":"reset_state","reason":"clear","state":{"forged":true}}}})).is_err());
}
