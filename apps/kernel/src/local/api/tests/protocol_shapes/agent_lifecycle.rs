//! MP-08/MP-09/MP-10/MP-11 A02/A03: protocol 561 agent task and wake projection snapshot and hash.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentObligation, AgentTaskExecution, AgentWait, AgentWake, ExecutionState,
};
use sha2::{Digest, Sha256};

fn room_session() -> crate::session::RuntimeSession {
    crate::session::RuntimeSession::new("room", None, "workspace", "worktree", "machine", "daemon")
}

#[test]
fn agent_task_projection_shape_is_bound_to_protocol_561() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 561);
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
            "status": "open", "dispatch_state": "accepted"}],
        "wait": {"registration_ids": ["reg"], "deadline_ms": 60000, "started_at_ms": 1,
            "inbox_cursor": 3, "long_wait_notified": false, "last_checked_at_ms": 1},
        "last_progress_at_ms": 1, "progress_sequence": 0, "no_progress_wakes": 1,
        "correction_used": false, "pending_prompt_id": null,
    });
    assert_eq!(value, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "4c248a2999981d5595b55343de0a317927e6e07a4eecb15bb2f85c076903aac1"
    );
}

#[test]
fn agent_wake_projection_shape_is_bound_to_protocol_561() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 561);
    let wake = AgentWake {
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
        "summary": "every 60s", "detail": {"interval_ms": 60000},
        "fire_count": 1, "missed_fires": 0, "last_fired_at_ms": 3, "last_sequence": 4,
        "last_delivery": "accepted", "last_delivered_at_ms": 5, "last_acknowledged_at_ms": null,
        "alerted_sequence": null,
    });
    assert_eq!(value, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "e3177f423a1bd3ff891a46566b5e2735a32bf6c1eec9f79c6cc7d31bebe64ed7"
    );
}

#[test]
fn an_empty_ledger_leaves_the_session_wire_shape_unchanged() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 561);
    let value = serde_json::to_value(room_session()).unwrap();
    assert!(value.get("agent_tasks").is_none() && value.get("agent_wakes").is_none());
}
