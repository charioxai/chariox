use crate::local::*;
use crate::session::{
    WorkflowDefinition, WorkflowOrigin, WorkflowOriginReason, WorkflowOriginSurface,
};

#[test]
fn agent_workflow_shapes_are_versioned_and_record_their_origin() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );

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
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );

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
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
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
    agent.set_parent_agent(Some(crate::agent::AgentRef::local("parent")));
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
    let snapshot = serde_json::json!({"agent": encoded[0]["parent_agent_id"], "workflow": encoded[1]["created_by_agent_id"], "run": encoded[2]["created_by_agent_id"], "queue_item": encoded[3]["created_by_agent_id"]});
    assert_eq!(
        snapshot,
        serde_json::json!({"agent":{"agent_id":"parent"},"workflow":"peer","run":"parent","queue_item":"parent"})
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "a8c897e7d8868e1e7c82a818feda25ffe687d7a8e75931370e0d2ef93d565054"
    );
    let mut legacy = encoded[0].clone();
    legacy.as_object_mut().unwrap().remove("parent_agent_id");
    let restored: crate::agent::AgentInstance = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.parent_agent_id(), None);
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
        .parent_agent_id()
        .is_none());
}

/// Protocol 1000: the parent edge, the spawn key and the user-only SetAgentParent.
#[test]
fn the_parent_edge_and_set_agent_parent_have_their_shapes() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let request =
        crate::local::LocalDaemonRequest::SetAgentParent(crate::local::SetAgentParentRequest {
            session_id: "room".into(),
            agent_id: "child".into(),
            parent_agent_id: Some("parent".into()),
        });
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"SetAgentParent":{"session_id":"room","agent_id":"child","parent_agent_id":"parent"}})
    );
    let none: crate::local::LocalDaemonRequest = serde_json::from_value(
        serde_json::json!({"SetAgentParent":{"session_id":"room","agent_id":"child","parent_agent_id":null}}),
    )
    .unwrap();
    assert!(matches!(
        none,
        crate::local::LocalDaemonRequest::SetAgentParent(r) if r.parent_agent_id.is_none()
    ));
    assert!(serde_json::from_value::<crate::local::LocalDaemonRequest>(
        serde_json::json!({"SetAgentParent":{"session_id":"room","agent_id":"child","parent":"x"}})
    )
    .is_err());
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
    let root = serde_json::to_value(&agent).unwrap();
    assert!(root.get("parent_agent_id").is_none() && root.get("spawn_key").is_none());
    agent.set_parent_agent(Some(crate::agent::AgentRef::local("parent")));
    agent.set_spawn_key(Some("spawn-1".into()));
    let child = serde_json::to_value(&agent).unwrap();
    assert_eq!(
        child["parent_agent_id"],
        serde_json::json!({"agent_id":"parent"})
    );
    assert_eq!(child["spawn_key"], "spawn-1");
    let restored: crate::agent::AgentInstance = serde_json::from_value(child).unwrap();
    assert_eq!(
        (restored.parent_agent_id(), restored.spawn_key()),
        (Some("parent"), Some("spawn-1"))
    );
}

/// Protocol 1000: the sub-agent tree read and its change event.
#[test]
fn the_agent_tree_read_and_change_event_have_their_shapes() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let request: crate::local::LocalDaemonRequest = serde_json::from_value(
        serde_json::json!({"ListDescendants":{"session_id":"room","agent_id":"root"}}),
    )
    .unwrap();
    assert!(matches!(
        request,
        crate::local::LocalDaemonRequest::ListDescendants(r) if r.agent_id == "root"
    ));
    let tree = crate::local::AgentTree {
        root: "root".into(),
        rev: 7,
        totals: crate::local::AgentTreeTotals {
            live: 1,
            running: 0,
            needs_user: 1,
        },
        nodes: vec![crate::local::AgentTreeNode {
            agent_id: "child".into(),
            parent_agent_id: Some("root".into()),
            kernel_id: None,
            alias: Some("worker".into()),
            provider: "codex".into(),
            model: None,
            execution_mode: Some(crate::provider::AgentExecutionMode::Plan),
            permission_level: Some(crate::provider::AgentPermissionLevel::Required),
            effort: Some("high".into()),
            account_profile: Some("work".into()),
            workspace_id: Some("app".into()),
            worktree_id: Some("app-2".into()),
            worker_kernel_id: Some("worker".into()),
            depth: 1,
            state: crate::local::AgentTreeState::Waiting,
            outcome: None,
            needs_user: true,
            deadline_ms: None,
            created_at_ms: 5,
        }],
    };
    assert_eq!(
        serde_json::to_value(crate::local::LocalDaemonResponse::AgentTree { tree }).unwrap(),
        serde_json::json!({"AgentTree":{"tree":{"root":"root","rev":7,
            "totals":{"live":1,"running":0,"needs_user":1},
            "nodes":[{"agent_id":"child","parent_agent_id":"root","kernel_id":null,
                "alias":"worker","provider":"codex","model":null,"execution_mode":"plan",
                "permission_level":"required","effort":"high","account_profile":"work",
                "workspace_id":"app","worktree_id":"app-2","worker_kernel_id":"worker","depth":1,
                "state":"waiting","outcome":null,"needs_user":true,"deadline_ms":null,"created_at_ms":5}]}}})
    );
    assert_eq!(
        serde_json::to_value(crate::local::OutsideEventValue::AgentTree {
            root: "root".into(),
            rev: 9
        })
        .unwrap(),
        serde_json::json!({"type":"agent_tree","value":{"root":"root","rev":9}})
    );
}

/// Protocol 1000: a sub-agent's decision is one card; a parent may answer it and the user's copy names the panel.
#[test]
fn a_sub_agent_decision_has_its_shapes() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        serde_json::to_value(crate::local::EventResolutionVia::ParentAgent).unwrap(),
        serde_json::json!("parent_agent")
    );
    assert_eq!(
        serde_json::to_value(crate::local::OutsideEventValue::Interaction {
            state: crate::local::EventInteractionState::Pending,
            pointer: None,
            question: None,
            resolution: None,
            shown_in: Some("room-agent".into()),
        })
        .unwrap(),
        serde_json::json!({"type": "interaction", "value": {"state": "pending", "shown_in": "room-agent"}})
    );
    assert_eq!(
        serde_json::to_value(crate::local::OutsideEventValue::Interaction {
            state: crate::local::EventInteractionState::Resolved,
            pointer: None,
            question: None,
            resolution: Some(crate::local::EventResolution {
                via: crate::local::EventResolutionVia::ParentAgent,
                choice_id: Some("allow".into()),
            }),
            shown_in: None,
        })
        .unwrap(),
        serde_json::json!({"type": "interaction", "value": {"state": "resolved", "resolution": {"via": "parent_agent", "choice_id": "allow"}}})
    );
}
