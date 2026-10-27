//! MP-08 / MP-10: the initial-view contract shares the allocated 508/116 bump.
use super::*;

#[test]
fn local_daemon_protocol_initial_session_view_shape_is_versioned_and_hashed() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let request = LocalDaemonRequest::AttachSessionView(crate::local::AttachSessionViewRequest {
        session_id: "session-1".into(),
        client_id: "client-1".into(),
        capability_level: ClientCapabilityLevel::FullTerminal,
        preferred_agent_id: Some("agent-7".into()),
        latest_prompt_count: None,
    });
    let request = serde_json::to_value(request).unwrap();
    assert_eq!(
        request,
        serde_json::json!({"AttachSessionView":{
            "session_id":"session-1", "client_id":"client-1",
            "capability_level":"FullTerminal", "preferred_agent_id":"agent-7",
        }})
    );
    let snapshot = serde_json::json!({
        "request": request, "response_fields": ["attachment", "history", "state"],
        "state_variant": "SessionState", "history_variant": "SessionHistoryOutline",
    });
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "a42cb81f327f7184db950f023fbc9d5ffd1b267d8040d0c4a5e11f7620e50d38"
    );
}

#[test]
fn local_daemon_protocol_initial_session_view_prompt_count_596_shape() {
    const _: () = assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 596);
    let request = LocalDaemonRequest::AttachSessionView(crate::local::AttachSessionViewRequest {
        session_id: "session-1".into(),
        client_id: "client-1".into(),
        capability_level: ClientCapabilityLevel::FullTerminal,
        preferred_agent_id: None,
        latest_prompt_count: Some(8),
    });
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({"AttachSessionView":{
            "session_id":"session-1", "client_id":"client-1",
            "capability_level":"FullTerminal", "preferred_agent_id":null,
            "latest_prompt_count":8,
        }})
    );
    // A client that does not ask sends no field, and the kernel reads that as one turn.
    let without: LocalDaemonRequest =
        serde_json::from_value(serde_json::json!({"AttachSessionView":{
            "session_id":"session-1", "client_id":"client-1", "capability_level":"FullTerminal",
        }}))
        .unwrap();
    let LocalDaemonRequest::AttachSessionView(without) = without else {
        panic!("expected attach");
    };
    assert_eq!(without.latest_prompt_count, None);
}
