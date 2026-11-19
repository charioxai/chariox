//! Local 1011: a Room's session carries the messages other agents queued for an agent's next turn,
//! and the person cancels or sends one with the queued-prompt requests, by its `agent-message-<n>` id.
use super::*;

#[test]
fn queued_agent_messages_protocol_1011_shape_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let queued = serde_json::json!([{
        "id": "agent-message-7",
        "agent_id": "agent-1",
        "sender": "worker-1",
        "prompt": "parser.rs is mine for the hour"
    }]);
    let mut session = crate::session::RuntimeSession::new(
        "session-1",
        None,
        "workspace-1",
        "worktree-1",
        "machine-1",
        "daemon-1",
    );
    assert_eq!(
        serde_json::to_value(&session)
            .unwrap()
            .get("queued_agent_messages"),
        None
    );
    session.set_queued_agent_messages(serde_json::from_value(queued.clone()).unwrap());
    assert_eq!(
        serde_json::to_value(&session).unwrap()["queued_agent_messages"],
        queued
    );
    let cancel = serde_json::json!({"CancelQueuedPrompt":{"attachment_id":"attachment-1","prompt_id":"agent-message-7","session_id":"session-1","target_agent_id":"agent-1"}});
    let send = serde_json::json!({"SteerQueuedPrompt":{"attachment_id":"attachment-1","prompt_id":"agent-message-7","session_id":"session-1","target_agent_id":"agent-1"}});
    for request in [&cancel, &send] {
        let value: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(&serde_json::to_value(value).unwrap(), request);
    }
    let snapshot = serde_json::json!({"local": LOCAL_DAEMON_PROTOCOL_VERSION,
        "peer": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        "shapes": {"queued": queued, "cancel": cancel, "send": send}});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&snapshot).unwrap().as_bytes())
        ),
        "2334a24cf468eb7af0a145db35c839493bdeb277714e87709a5c1c1c3b295938"
    );
}
