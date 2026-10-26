//! MP-08/MP-09/MP-10/MP-11 A02/A03: protocol 561 agent task and wake projection snapshot and hash.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentObligation, AgentTaskExecution, AgentWait, ExecutionState,
};
use sha2::{Digest, Sha256};

fn room_session() -> crate::session::RuntimeSession {
    crate::session::RuntimeSession::new("room", None, "workspace", "worktree", "machine", "daemon")
}

#[test]
fn agent_task_projection_shape_is_bound_to_protocol452() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let task = AgentTaskExecution {
        goal: None,
        task_id: "task".into(),
        room_id: "room".into(),
        owner_user_id: "owner".into(),
        agent_id: "agent".into(),
        prompt_id: "turn".into(),
        provider_run_id: Some("run".into()),
        revision: 2,
        blocked_revision: 0,
        state: ExecutionState::Waiting,
        reason: "delegate result".into(),
        obligations: vec![AgentObligation {
            id: "obligation".into(),
            kind: "delegate".into(),
            resource_id: Some("child".into()),
            completion_task_id: Some("child-task".into()),
            status: "open".into(),
            dispatch_state: "accepted".into(),
        }],
        wait: Some(AgentWait {
            registration_ids: vec!["reg".into()],
            deadline_ms: 60_000,
            started_at_ms: 1,
            inbox_cursor: 3,
            long_wait_notified: false,
            last_checked_at_ms: 1,
        }),
        last_progress_at_ms: 1,
        progress_sequence: 0,
        no_progress_wakes: 1,
        correction_used: false,
        pending_prompt_id: None,
    };
    let mut session = room_session();
    session.set_agent_tasks(vec![task.clone()]);
    let value = serde_json::to_value(task).unwrap();
    assert_eq!(
        serde_json::to_value(&session).unwrap()["agent_tasks"],
        serde_json::json!([value.clone()])
    );
    let expected = serde_json::json!({
        "task_id": "task", "room_id": "room", "owner_user_id": "owner", "agent_id": "agent",
        "prompt_id": "turn", "provider_run_id": "run", "revision": 2, "blocked_revision": 0,
        "state": "waiting", "reason": "delegate result",
        "obligations": [{"id": "obligation", "kind": "delegate", "resource_id": "child",
            "completion_task_id": "child-task", "status": "open", "dispatch_state": "accepted"}],
        "wait": {"registration_ids": ["reg"], "deadline_ms": 60000, "started_at_ms": 1,
            "inbox_cursor": 3, "long_wait_notified": false, "last_checked_at_ms": 1},
        "last_progress_at_ms": 1, "progress_sequence": 0, "no_progress_wakes": 1,
        "correction_used": false, "pending_prompt_id": null,
    });
    assert_eq!(value, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "1200a129c1634205e402b26eed546a5418aca143b91457f416e67438959e96af"
    );
}

#[test]
fn agent_wake_projection_shape_is_bound_to_protocol459() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let wake = crate::durable_state::agent_lifecycle::AgentWake {
        id: "wake".into(),
        task_id: "task".into(),
        room_id: "room".into(),
        agent_id: "agent".into(),
        registration_id: "reg".into(),
        kind: "timer".into(),
        label: "check-in".into(),
        state: "scheduled".into(),
        created_at_ms: 1,
        verified_at_ms: Some(2),
        next_due_ms: Some(60_001),
        interval_ms: Some(60_000),
        command: vec![],
        match_text: None,
        matched_at_ms: None,
        pid: None,
        exit_code: None,
        summary: "every 60s".into(),
        detail: serde_json::json!({"interval_ms": 60000}),
        fire_count: 1,
        missed_fires: 0,
        last_fired_at_ms: Some(3),
        last_sequence: Some(4),
        last_delivery: Some("accepted".into()),
        last_delivered_at_ms: Some(5),
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    };
    let mut session = room_session();
    session.set_agent_wakes(vec![wake.clone()]);
    let value = serde_json::to_value(wake).unwrap();
    assert_eq!(
        serde_json::to_value(&session).unwrap()["agent_wakes"],
        serde_json::json!([value.clone()])
    );
    let expected = serde_json::json!({
        "id": "wake", "task_id": "task", "room_id": "room", "agent_id": "agent",
        "registration_id": "reg", "kind": "timer", "label": "check-in", "state": "scheduled",
        "created_at_ms": 1, "verified_at_ms": 2, "next_due_ms": 60001, "interval_ms": 60000,
        "command": [], "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
        "summary": "every 60s", "detail": {"interval_ms": 60000},
        "fire_count": 1, "missed_fires": 0, "last_fired_at_ms": 3, "last_sequence": 4,
        "last_delivery": "accepted", "last_delivered_at_ms": 5, "last_acknowledged_at_ms": null,
        "alerted_sequence": null,
    });
    assert_eq!(value, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "70b6f79b62077c01451c2c12808fef81d8239f20f72bf749d84b068cf018518c"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 A12: coordinator-assigned union 493.
#[test]
fn goal_projection_shape_is_bound_to_assigned_protocol493() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version(),
        "amint must fold the goal projection into the assigned union"
    );
    let expected = serde_json::json!({"schema_version":1,"id":"goal:p","owner_user_id":"owner","room_id":"room","agent_id":"agent","objective":"Verify artifacts","original_prompt_id":"p","objective_revision":1,"created_at_ms":1,"active_elapsed_ms":2,"waiting_elapsed_ms":3,"state_since_ms":4,"check_in_minutes":10,"next_check_in_ms":null,"paused":false,"continuation_sequence":1,"check_count":1,"evaluation":{"origin":"completion_check","prompt_id":"check","work_prompt_id":"p","work_provider_run_id":"run","objective_revision":1,"ledger_revision":3,"started_at_ms":4,"deadline_ms":60004,"result":{"status":"continue","reason":"independent work"}},"final_answer":null,"evidence":[]});
    let goal: crate::durable_state::agent_lifecycle::goals::AgentGoal =
        serde_json::from_value(expected.clone()).unwrap();
    let value = serde_json::to_value(goal).unwrap();
    assert_eq!(value, expected);
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "d6e7a96c9b733533436b6434d32bb3036bc90cfef876c0f413b2cc8affb6a78b"
    );
    assert_eq!(
        serde_json::to_value(crate::session::PromptOrigin::CompletionCheck).unwrap(),
        serde_json::json!("completion_check")
    );
}

// MP-08 / MP-09 / MP-10 / MP-11 A12: precise additions for coordinator union493/102.
#[test]
fn compact_task_wire_shapes_are_bound_to_assigned_protocol493() {
    let raw = serde_json::json!({"task_id":"task","room_id":"room","owner_user_id":"owner","agent_id":"agent","prompt_id":"prompt","provider_run_id":null,"revision":2,"blocked_revision":0,"state":"working","reason":"work","obligations":[],"wait":null,"last_progress_at_ms":1,"progress_sequence":0,"no_progress_wakes":0,"correction_used":false,"pending_prompt_id":null});
    let task = serde_json::from_value(raw.clone()).unwrap();
    let response = serde_json::to_value(LocalDaemonResponse::AgentTaskUpdated {
        session_id: "room".into(),
        task: Box::new(task),
    })
    .unwrap();
    let task = serde_json::from_value(raw.clone()).unwrap();
    let event = serde_json::to_value(
        crate::transport::kernel_protocol::KernelEvent::AgentTaskUpdated {
            session_id: "room".into(),
            task: Box::new(task),
        },
    )
    .unwrap();
    assert_eq!(
        response,
        serde_json::json!({"AgentTaskUpdated":{"session_id":"room","task":raw}})
    );
    assert_eq!(
        event,
        serde_json::json!({"event":"agent_task_updated","session_id":"room","task":raw})
    );
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!("{:x}", Sha256::digest(response.to_string().as_bytes())),
        "4c0a03a004a412d708a4d60c1475b4838001ee1a9437501f64605a4e9c939869"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(event.to_string().as_bytes())),
        "8a4f8a22a07cd18fe4ca79ce9277dc9dd44bb73330c2a3f85bf0b6102ba2d4e4"
    );
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version(),
        "amint must fold compact task frames into the assigned union"
    );
}

// MP-08 / MP-10 / MP-11: protocol626 Stop may cancel a task after its prompt ended.
#[test]
fn parked_stop_response_semantics_are_versioned() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert!(crate::test_support::expected_local_protocol_version() >= 626);
    let request = LocalDaemonRequest::CancelActivePrompt(crate::local::CancelActivePromptRequest {
        session_id: "room".into(),
        attachment_id: "terminal".into(),
        target_agent_id: Some("agent".into()),
        expected_prompt_id: None,
    });
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("parked-stop-626.json")).unwrap();
    let response: LocalDaemonResponse =
        serde_json::from_value(expected["response"].clone()).unwrap();
    let value = serde_json::json!({"request":request, "response":response});
    assert_eq!(value, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "5d4d7cab46a07c8f05fa4223b2853b66cb117a99a13ccd4c68446d87b2e4d731"
    );
}
