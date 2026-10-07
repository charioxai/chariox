//! MP-08 / MP-09 / MP-10 / MP-11 A02: local452 durable task projection guard.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, AgentWait, ExecutionState};
#[test]
fn agent_task_projection_shape_is_bound_to_protocol452() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 452);
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
        obligations: vec![],
        wait: Some(AgentWait {
            registration_ids: vec!["reg".into()],
            deadline_ms: 60_000,
            started_at_ms: 1,
            inbox_cursor: 3,
            long_wait_notified: false,
        }),
        last_progress_at_ms: 1,
        progress_sequence: 0,
        no_progress_wakes: 1,
        correction_used: false,
        pending_prompt_id: None,
    };
    let value = serde_json::to_value(task).unwrap();
    let expected = serde_json::json!({"task_id":"task","room_id":"room","owner_user_id":"owner","agent_id":"agent","prompt_id":"turn","provider_run_id":"run","revision":2,"blocked_revision":0,"state":"waiting","reason":"delegate result","obligations":[],"wait":{"registration_ids":["reg"],"deadline_ms":60000,"started_at_ms":1,"inbox_cursor":3,"long_wait_notified":false},"last_progress_at_ms":1,"progress_sequence":0,"no_progress_wakes":1,"correction_used":false,"pending_prompt_id":null});
    assert_eq!(value, expected);
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
        "6bcc4e301f4ff72d075eb60dd595055bbcc4e949419f425c4763fb5f13b93669"
    );
}
