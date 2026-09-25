use crate::local::*;
use crate::session::{
    WorkflowDefinition, WorkflowOrigin, WorkflowOriginReason, WorkflowOriginSurface,
};

#[test]
fn agent_workflow_shapes_are_versioned_and_record_their_origin() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 352);

    let request = LocalDaemonRequest::CreateAgentWorkflow(CreateAgentWorkflowRequest {
        session_id: "session-1".into(),
        agent_id: "agent-1".into(),
        reason: WorkflowOriginReason::Trigger,
        surface: WorkflowOriginSurface::Web,
        alias: None,
    });
    let encoded = serde_json::json!({"CreateAgentWorkflow": {
        "session_id": "session-1", "agent_id": "agent-1",
        "reason": "trigger", "surface": "web", "alias": null
    }});
    assert_eq!(serde_json::to_value(&request).unwrap(), encoded);
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({"CreateAgentWorkflow": {
            "session_id": "session-1", "agent_id": "agent-1", "reason": "deploy", "surface": "tui"
        }}))
        .unwrap(),
        LocalDaemonRequest::CreateAgentWorkflow(CreateAgentWorkflowRequest {
            session_id: "session-1".into(),
            agent_id: "agent-1".into(),
            reason: WorkflowOriginReason::Deploy,
            surface: WorkflowOriginSurface::Tui,
            alias: None,
        })
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"CreateAgentWorkflow": {
            "session_id": "s", "agent_id": "a", "reason": "bind", "surface": "web"
        }})
    )
    .is_err());

    let mut workflow = WorkflowDefinition::new("workflow-1", Some("builder-trigger".into()));
    // A workflow without an origin serializes exactly as before.
    assert!(serde_json::to_value(&workflow)
        .unwrap()
        .get("origin")
        .is_none());
    workflow.set_origin(WorkflowOrigin {
        source_agent_id: "agent-1".into(),
        reason: WorkflowOriginReason::Trigger,
        surface: WorkflowOriginSurface::Web,
        created_at_ms: 7,
    });
    assert_eq!(
        serde_json::to_value(&workflow).unwrap()["origin"],
        serde_json::json!({"source_agent_id": "agent-1", "reason": "trigger", "surface": "web", "created_at_ms": 7})
    );
}
