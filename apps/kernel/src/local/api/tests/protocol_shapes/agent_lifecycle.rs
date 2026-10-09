//! MP-08 / MP-09 / MP-10 / MP-11 A02: local452 durable task projection guard.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentObligation, AgentTaskExecution, AgentWait, ExecutionState,
};
#[test]
fn agent_task_projection_shape_is_bound_to_protocol459() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 459);
    let task = AgentTaskExecution {
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
    // The session snapshot field name is part of the 452 shape too.
    let mut session = crate::session::RuntimeSession::new(
        "room",
        None,
        "workspace",
        "worktree",
        "machine",
        "daemon",
    );
    session.set_agent_tasks(vec![task.clone()]);
    let projected = serde_json::to_value(&session).unwrap();
    let value = serde_json::to_value(task).unwrap();
    assert_eq!(projected["agent_tasks"], serde_json::json!([value.clone()]));
    let expected = serde_json::json!({"task_id":"task","room_id":"room","owner_user_id":"owner","agent_id":"agent","prompt_id":"turn","provider_run_id":"run","revision":2,"blocked_revision":0,"state":"waiting","reason":"delegate result","obligations":[{"id":"obligation","kind":"delegate","resource_id":"child","completion_task_id":"child-task","status":"open","dispatch_state":"accepted"}],"wait":{"registration_ids":["reg"],"deadline_ms":60000,"started_at_ms":1,"inbox_cursor":3,"long_wait_notified":false,"last_checked_at_ms":1},"last_progress_at_ms":1,"progress_sequence":0,"no_progress_wakes":1,"correction_used":false,"pending_prompt_id":null});
    assert_eq!(value, expected);
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "1200a129c1634205e402b26eed546a5418aca143b91457f416e67438959e96af"
    );
}

#[test]
fn agent_wake_projection_shape_is_bound_to_protocol459() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 459);
    let wake = crate::durable_state::agent_lifecycle::AgentWake {
        id: "wake".into(),
        task_id: "task".into(),
        room_id: "room".into(),
        agent_id: "agent".into(),
        registration_id: "completion-wake".into(),
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
        fire_count: 1,
        missed_fires: 0,
        last_fired_at_ms: Some(3),
        last_sequence: Some(4),
        last_delivery: Some("accepted".into()),
        last_delivered_at_ms: Some(5),
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    };
    let mut session = crate::session::RuntimeSession::new(
        "room",
        None,
        "workspace",
        "worktree",
        "machine",
        "daemon",
    );
    session.set_agent_wakes(vec![wake.clone()]);
    let value = serde_json::to_value(wake).unwrap();
    assert_eq!(
        serde_json::to_value(&session).unwrap()["agent_wakes"],
        serde_json::json!([value.clone()])
    );
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "6c53bc5bc2450a64e83266d837cfc7a00ced03affeb07697841e1f220d067462"
    );
}
