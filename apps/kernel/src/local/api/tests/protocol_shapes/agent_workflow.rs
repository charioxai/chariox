use crate::local::*;
use crate::session::{
    WorkflowDefinition, WorkflowOrigin, WorkflowOriginReason, WorkflowOriginSurface,
};

#[test]
fn agent_workflow_shapes_are_versioned_and_record_their_origin() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 486);

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

/// Protocol 397: substitutes rerun one failed turn, so there is no action to
/// select one and no active-substitute state on the agent.
#[test]
fn agent_substitute_shape_is_per_turn_only() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 486);

    for retired in [
        serde_json::json!({"Activate": {"index": 0, "reason": "manual"}}),
        serde_json::json!({"Primary": {}}),
    ] {
        assert!(
            serde_json::from_value::<AgentSubstituteAction>(retired.clone()).is_err(),
            "{retired} is retired"
        );
    }
    for action in [
        serde_json::json!({"Remove": {"index": 0}}),
        serde_json::json!({"Move": {"from_index": 1, "to_index": 0}}),
        serde_json::json!({"Clear": {}}),
        serde_json::json!({"SetTimeout": {"timeout_ms": 30000}}),
    ] {
        let parsed = serde_json::from_value::<AgentSubstituteAction>(action.clone())
            .expect("list action parses");
        assert_eq!(serde_json::to_value(parsed).unwrap(), action);
    }

    let mut agent = crate::agent::AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "codex",
        Some("gpt-6.1-sol".into()),
        Some("high".into()),
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    agent.add_substitute(crate::agent::AgentSubstituteProfile::new(
        "claude",
        "claude-opus-5-5",
        None,
    ));
    let encoded = serde_json::to_value(&agent).expect("agent encodes");
    let fields = encoded.as_object().expect("agent is an object");
    for retired in [
        "primary_provider",
        "primary_model",
        "primary_effort",
        "primary_account_profile",
        "active_substitute_index",
        "last_substitution",
    ] {
        assert!(!fields.contains_key(retired), "{retired} is retired");
    }
    assert_eq!(
        encoded.pointer("/substitutes/0"),
        Some(&serde_json::json!({"provider": "claude", "model": "claude-opus-5-5"}))
    );
}

// MP-08 / MP-11, A01: protocol 450 persists authoritative creator lineage.
#[test]
fn room_creator_lineage_protocol_450_snapshot() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 486);
    let mut agent = crate::agent::AgentInstance::new(
        "child",
        "ref",
        "room",
        None,
        "codex",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    agent.record_spawn_creator(Some("parent".into()));
    agent.set_controlled_by_metaagent_id(Some("unrelated-controller".into()));
    let definition = WorkflowDefinition::new_controlled_by_metaagent("definition", None, "peer");
    let run = crate::session::WorkflowRun::new(
        "run",
        "definition",
        "endpoint",
        "node",
        None,
        None,
        vec![],
        vec![],
    )
    .with_creator(Some("parent".into()));
    let queued =
        crate::session::WorkflowQueuedPrompt::new(crate::session::WorkflowQueuedPromptInput {
            id: "queue-item".into(),
            queue_id: "queue".into(),
            workflow_id: "definition".into(),
            endpoint_id: "endpoint".into(),
            prompt: None,
            publication_invocation: None,
            source: crate::session::WorkflowQueuedPromptSource::Manual,
            schedule_id: None,
        })
        .with_creator(Some("parent".into()));
    let encoded = [
        serde_json::to_value(&agent).unwrap(),
        serde_json::to_value(&definition).unwrap(),
        serde_json::to_value(&run).unwrap(),
        serde_json::to_value(&queued).unwrap(),
    ];
    let snapshot = serde_json::json!({"agent": encoded[0]["spawned_by_agent_id"], "workflow": encoded[1]["created_by_agent_id"], "run": encoded[2]["created_by_agent_id"], "queue_item": encoded[3]["created_by_agent_id"]});
    assert_eq!(
        snapshot,
        serde_json::json!({"agent":"parent","workflow":"peer","run":"parent","queue_item":"parent"})
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "8ff2bd21b211626c4c599da9d01565f6ea8caa69c6c95d8b7e6d81f8d60d0de5"
    );
    let mut legacy = encoded[0].clone();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("spawned_by_agent_id");
    let restored: crate::agent::AgentInstance = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.spawned_by_agent_id(), None);
    for (value, key) in encoded[1..].iter().zip(["created_by_agent_id"; 3]) {
        let mut legacy = value.clone();
        legacy.as_object_mut().unwrap().remove(key);
        assert!(legacy.get(key).is_none());
        // Old definition/run/queue snapshots deserialize without inventing an actor.
        match legacy["id"].as_str().unwrap() {
            "definition" => assert!(serde_json::from_value::<WorkflowDefinition>(legacy)
                .unwrap()
                .created_by_agent_id()
                .is_none()),
            "run" => assert!(
                serde_json::from_value::<crate::session::WorkflowRun>(legacy)
                    .unwrap()
                    .created_by_agent_id()
                    .is_none()
            ),
            _ => assert!(
                serde_json::from_value::<crate::session::WorkflowQueuedPrompt>(legacy)
                    .unwrap()
                    .created_by_agent_id()
                    .is_none()
            ),
        }
    }
    assert!(agent
        .materialized_for_workflow_runtime("copy", "copy-ref", "room", "worktree")
        .spawned_by_agent_id()
        .is_none());
}
