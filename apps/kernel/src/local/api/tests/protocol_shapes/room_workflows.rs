use super::*;
use sha2::{Digest, Sha256};

#[test]
fn room_workflows_protocol_439_shapes_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 439);
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
