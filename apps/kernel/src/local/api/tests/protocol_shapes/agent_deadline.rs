//! MP-08 / MP-09 / MP-10 / MP-11: executor-local durable deadlines.
use super::*;
#[test]
fn agent_deadline_protocol_shape_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let expected = serde_json::json!({"SetAgentDeadline":{"agent_id":"agent","deadline_ms":100,"session_id":"room"}});
    let value: LocalDaemonRequest = serde_json::from_value(expected.clone()).unwrap();
    let actual = serde_json::to_value(value).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&actual).unwrap().as_bytes())
        ),
        "cae2737c0248e13c9eb61a27fa2dbea014fbff024a54c5cee7f0bc146488d14e"
    );
    let expected = serde_json::json!({"GetAgentDeadline":{"agent_id":"agent","session_id":"room"}});
    let value: LocalDaemonRequest = serde_json::from_value(expected.clone()).unwrap();
    let actual = serde_json::to_value(value).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&actual).unwrap().as_bytes())
        ),
        "c6674d17a11e2a932807a7a53627419707600a81506073e3aca6cca3bbddde06"
    );
    let expected = serde_json::json!({"AgentDeadline":{"deadline":{"agent_id":"agent","deadline_ms":100,"reached_at_ms":101,"session_id":"room","state":"reached","stopped_at_ms":102}}});
    let value: LocalDaemonResponse = serde_json::from_value(expected.clone()).unwrap();
    let actual = serde_json::to_value(value).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&actual).unwrap().as_bytes())
        ),
        "aeb95dfdd8722d16963616914163e5411f0314a5f9ffa643e34a807eb9bd48a8"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({"SetAgentDeadline":{"session_id":"room","agent_id":"agent","deadline_ms":100,"extra":true}})).is_err());
}

#[test]
fn deadline_clear_is_explicit_and_has_a_stable_shape() {
    let expected = serde_json::json!({"SetAgentDeadline":{"agent_id":"agent","deadline_ms":null,"session_id":"room"}});
    let request: LocalDaemonRequest = serde_json::from_value(expected.clone()).unwrap();
    let actual = serde_json::to_value(request).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "4b46174a6bf6e6f45eb71c82fe3e9f5afcd7296f7a74edaac27993a5d2bf2560"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"SetAgentDeadline":{"session_id":"room","agent_id":"agent"}})
    )
    .is_err());
}

#[test]
fn undelivered_deadline_message_outcome_is_versioned_and_hashed() {
    use crate::durable_state::agent_lifecycle::DelegateGoneReason;

    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let actual = DelegateGoneReason::AgentDeadlineReached.answer("agent");
    assert_eq!(
        actual,
        serde_json::json!({"delegate_id": "agent", "kind": "delegate_gone", "reason": "agent_deadline_reached"})
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "80a27c224bc71babeace7f3040b8105ff8480b86002556fe61c0fd7cece57805"
    );
}

#[test]
fn deadline_notice_reason_survives_operational_history_and_wire_projection() {
    use crate::history::{
        HistoryEvent, HistoryEventTurnContext, SessionHistoryEntry, SessionHistoryNoticeReason,
    };

    let mut notice =
        SessionHistoryEntry::notice("room", None, Some("sender"), "Start a new agent.");
    notice.notice_reason = Some(SessionHistoryNoticeReason::AgentDeadlineReached);
    let context = HistoryEventTurnContext {
        session_id: Some("room".into()),
        ..Default::default()
    };
    let recorded = HistoryEvent::transcript(1, &notice, context);
    let restored = recorded.to_session_history_entry().unwrap();
    assert_eq!(restored.notice_reason, notice.notice_reason);
    let wire = serde_json::to_value(restored).unwrap();
    let shape = serde_json::json!({"kind": wire["kind"], "notice_reason": wire["notice_reason"]});
    assert_eq!(
        shape,
        serde_json::json!({"kind":"notice","notice_reason":"agent_deadline_reached"})
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&shape).unwrap())),
        "acdd8edbb2ec54bfc38e4c79c42c000af67e4165691812f536e719b0575c2f9f"
    );
}
