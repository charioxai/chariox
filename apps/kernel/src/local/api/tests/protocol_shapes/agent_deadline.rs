//! MP-08 / MP-09 / MP-10 / MP-11: executor-local durable deadlines.
use super::*;
#[test]
fn lane_deadline_protocol_shape_and_hash() {
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
