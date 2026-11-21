//! What an agent waits on and the agents the health report flags are part of the client wire.
use crate::runtime::projection::{AgentRuntimeActivity, AgentRuntimeProjectionHealthSnapshot};
use sha2::{Digest, Sha256};

#[test]
fn populated_agent_liveness_and_silent_agents_protocol_snapshot_and_hash() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("agent-liveness.json")).unwrap();
    let activity: AgentRuntimeActivity =
        serde_json::from_value(expected["activity"].clone()).unwrap();
    let health: AgentRuntimeProjectionHealthSnapshot =
        serde_json::from_value(expected["health"].clone()).unwrap();
    let actual = serde_json::json!({"activity": activity, "health": health});
    assert_eq!(actual, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "779368333283c6ba95ed8d8859d90c17ef81c0925a2ec27a1e6e8553bf0ca9e9"
    );
}

// Miguel 11-16: a waiting agent says what would wake it, and nothing judges how long it made no progress.
#[test]
fn waiting_events_wait_without_long_notice_and_question_task_are_bound_to_protocol1011() {
    use crate::durable_state::agent_lifecycle::{AgentTaskExecution, AgentWait, TaskOwnerAction};
    use crate::runtime::projection::AgentLiveness;
    let bytes = include_bytes!("agent-waiting-events-1011.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "1116e6954f7ae1b9af59226afb4047860b75441ae860350fa15a7f3680d0f7fb"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        fixture["local"],
        crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
    );
    assert_eq!(
        fixture["peer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let liveness: AgentLiveness = serde_json::from_value(fixture["liveness"].clone()).unwrap();
    assert_eq!(liveness.waiting_events.len(), 2);
    assert_eq!(
        liveness.waiting_events[1].cancel_wake_id.as_deref(),
        Some("wake-9")
    );
    assert_eq!(serde_json::to_value(liveness).unwrap(), fixture["liveness"]);
    let wait: AgentWait = serde_json::from_value(fixture["wait"].clone()).unwrap();
    let wire = serde_json::to_value(wait).unwrap();
    assert_eq!(wire, fixture["wait"]);
    assert!(wire.get("long_wait_notified").is_none());
    let task: AgentTaskExecution =
        serde_json::from_value(fixture["question_task"].clone()).unwrap();
    assert_eq!(task.owner_action, Some(TaskOwnerAction::Question));
    assert_eq!(
        serde_json::to_value(task).unwrap(),
        fixture["question_task"]
    );
}
