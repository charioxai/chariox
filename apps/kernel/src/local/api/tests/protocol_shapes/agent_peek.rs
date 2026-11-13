//! MP-08 / MP-10 / MP-11 Landing 10 peek (local 1006, peer 445): an agent's transcript
//! as titled rows and one row by pages.
use super::*;

#[test]
fn agent_peek_protocol_1006_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let peek = serde_json::json!({"PeekAgentTranscript":{"agent_id":"agent","before":"42","limit":10,"session_id":"room"}});
    let read = serde_json::json!({"ReadAgentTranscriptEntry":{"agent_id":"agent","entry_id":"history:4:6","limit":4000,"offset":0,"session_id":"room"}});
    let peeked = serde_json::json!({"AgentTranscriptPeek":{"peek":{"agent_id":"agent","before":"4","entries":[
        {"at_ms":1000,"chars":120,"duration_ms":60000,"entry_id":"history:4:6","kind":"tool_call","status":"running","title":"Running cargo build"},
        {"at_ms":2000,"chars":30,"entry_id":"history:7:7","kind":"wait","status":"timed_out","title":"Timed out after 30 min 0 s"}]}}});
    let page = serde_json::json!({"AgentTranscriptEntryPage":{"page":{"entry_id":"history:4:6","kind":"tool_call","next_offset":4000,"offset":0,"text":"input: {}\n","title":"Running cargo build","total_chars":8000}}});
    for request in [&peek, &read] {
        let value: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(&serde_json::to_value(value).unwrap(), request);
    }
    for response in [&peeked, &page] {
        let value: LocalDaemonResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(&serde_json::to_value(value).unwrap(), response);
    }
    let snapshot = serde_json::json!({"local": LOCAL_DAEMON_PROTOCOL_VERSION,
        "peer": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        "shapes": {"peek": peek, "read": read, "peeked": peeked, "page": page}});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_string(&snapshot).unwrap().as_bytes())
        ),
        "813bae33ed4cd0d5780b782a72149642d2b9fd17d8c20cd53729634e3fbca60c"
    );
}
