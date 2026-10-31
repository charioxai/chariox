use super::*;
use sha2::{Digest, Sha256};

#[test]
fn room_workflows_protocol_439_shapes_are_versioned() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let room = crate::session::RuntimeSession::new(
        "room",
        None,
        "workspace",
        "worktree",
        "machine",
        "home",
    );
    let mut inventory = crate::runtime::projection::RoomWorkflowInventory::project(&room, "local");
    inventory
        .workflows
        .push(crate::runtime::projection::RoomWorkflowSummary {
            workflow_id: "workflow".into(),
            workflow_revision: 1,
            label: "Workflow".into(),
            state: crate::runtime::projection::RoomWorkflowState::Mixed,
            running_count: 1,
            paused_count: 1,
            queued_count: 1,
            endpoints: vec![crate::runtime::projection::RoomWorkflowEndpointSummary {
                endpoint_id: "endpoint".into(),
                label: "Entry".into(),
                entry_node_id: "node".into(),
                can_start: false,
                start_disabled_reason: Some("Owner only".into()),
            }],
            runs: vec![crate::runtime::projection::RoomWorkflowRunSummary {
                run_id: "captured".into(),
                created_before_ownership_tracking: false,
                endpoint_id: "endpoint".into(),
                status: crate::session::WorkflowRunStatus::Running,
                can_pause: true,
                can_resume: false,
                can_stop: true,
            }],
        });
    inventory.workflow_count = 1;
    let request =
        LocalDaemonRequest::ControlRoomWorkflowRuns(crate::local::ControlRoomWorkflowRunsRequest {
            session_id: "room".into(),
            workflow_id: "workflow".into(),
            action: crate::local::RoomWorkflowRunAction::Stop,
            run_ids: vec!["captured".into()],
        });
    let response = LocalDaemonResponse::RoomWorkflowRunsControlled {
        inventory: inventory.clone(),
        results: vec![crate::local::RoomWorkflowRunControlResult {
            run_id: "captured".into(),
            outcome: crate::local::RoomWorkflowRunControlOutcome::Unchanged,
            error: None,
        }],
    };
    let event = crate::transport::kernel_protocol::KernelEvent::RoomWorkflowsChanged { inventory };
    let shape = serde_json::json!({"request": request, "response": response, "event": event});
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&shape).unwrap())),
        "70e713e4004e92d4c741af4a4da84a07697841138c68fab3f1d1b1d0f42c5e7c"
    );
}

#[test]
fn mp08_mp11_session_state_and_snapshot_hash_required_room_workflows() {
    let room = crate::session::RuntimeSession::new(
        "room",
        None,
        "workspace",
        "worktree",
        "machine",
        "home",
    );
    let inventory = crate::runtime::projection::RoomWorkflowInventory::project(&room, "local");
    let state = LocalDaemonResponse::SessionState {
        room_workflows: inventory.clone(),
        session: room.clone(),
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 0,
        processes: Vec::new(),
    };
    let event = crate::transport::kernel_protocol::KernelEvent::SessionSnapshot {
        room_workflows: inventory,
        session: Box::new(room),
        provider_run: Box::new(None),
        agent_activity: Box::new(std::collections::BTreeMap::new()),
        agent_activity_revision: 0,
    };
    let mut state = serde_json::to_value(state).unwrap();
    let mut event = serde_json::to_value(event).unwrap();
    // Session timestamps and nested session fields have independent contracts;
    // this hash fences the required outer snapshot projection at protocol 491.
    state["SessionState"]["session"] = serde_json::Value::Null;
    event["session"] = serde_json::Value::Null;
    assert!(state["SessionState"]
        .as_object()
        .unwrap()
        .contains_key("room_workflows"));
    assert!(event.as_object().unwrap().contains_key("room_workflows"));
    let shape = serde_json::json!({"state":state,"event":event});
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&shape).unwrap())),
        "628cb03a194b3ab41a12f55d462e9b4cef488981355582dd32c3c7acbb67fe8e"
    );
}

// MP-08 / MP-10 / MP-11: bind the historical marker to its reviewed 513/121 snapshot.
#[test]
fn historical_workflow_projection_protocol_snapshot() {
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../fixtures/historical-workflow-protocol.json"
    ))
    .unwrap();
    assert_eq!(
        Some(u64::from(LOCAL_DAEMON_PROTOCOL_VERSION)),
        expected["local"].as_u64()
    );
    assert_eq!(
        Some(u64::from(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
        )),
        expected["relayPeer"].as_u64()
    );
    let mut run = crate::session::WorkflowRun::new(
        "historical",
        "workflow",
        "endpoint",
        "node",
        None,
        None,
        vec![],
        vec![],
    );
    run.mark_created_before_ownership_tracking();
    let summary = crate::runtime::projection::RoomWorkflowRunSummary {
        run_id: "historical".into(),
        endpoint_id: "endpoint".into(),
        status: crate::session::WorkflowRunStatus::Running,
        created_before_ownership_tracking: true,
        can_pause: false,
        can_resume: false,
        can_stop: false,
    };
    let mut shape = serde_json::json!({"run":run,"summary":summary});
    shape["run"]["created_at_ms"] = 0.into();
    shape["run"]["received_at_ms"] = 0.into();
    assert_eq!(shape, expected["shape"]);
}
