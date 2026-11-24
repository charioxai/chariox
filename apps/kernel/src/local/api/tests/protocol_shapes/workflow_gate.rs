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
    // A held entry names its account and the earliest reset; both ride the node run and are
    // absent from one that is not held.
    for wire in snapshot["node_run_held_entries"].as_array().unwrap() {
        let mut node_run = crate::session::WorkflowNodeRun::new(
            "node-run",
            "node",
            "agent",
            0,
            crate::session::WorkflowNodeRunStatus::Ready,
        );
        let free = serde_json::to_value(&node_run).unwrap();
        for field in ["waiting_reason", "waiting_account", "waiting_until_ms"] {
            assert!(free.get(field).is_none(), "{field} is absent when not held");
        }
        node_run.set_wait(crate::session::WorkflowNodeWait {
            reason: serde_json::from_value(wire["waiting_reason"].clone()).unwrap(),
            account: Some(wire["waiting_account"].as_str().unwrap().to_string()),
            resets_at_ms: wire["waiting_until_ms"].as_u64(),
        });
        let held = serde_json::to_value(&node_run).unwrap();
        for field in ["waiting_reason", "waiting_account", "waiting_until_ms"] {
            assert_eq!(held[field], wire[field], "{field}");
        }
        let read: crate::session::WorkflowNodeRun = serde_json::from_value(held.clone()).unwrap();
        assert_eq!(serde_json::to_value(read).unwrap(), held);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "8e943aa52d32abb8c7dafcfd69147aeb8160197c15f709467529ad6073c5b3b9"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({"WorkflowGate":{"session_id":"room","workflow_ref":"advisory","action":{"kind":"reset_state","reason":"clear","state":{"forged":true}}}})).is_err());
}
