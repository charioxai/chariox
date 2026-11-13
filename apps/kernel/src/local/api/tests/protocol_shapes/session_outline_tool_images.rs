use super::*;

// MP-08 / MP-10: local 1000 adds `imageCount` to a tool blob's outline preview, so a client
// knows to read a call whose result returned pictures without the outline carrying them.
const FIXTURE: &[u8] = include_bytes!("session-outline-tool-image-count-1000.json");

#[test]
fn mp10_session_outline_tool_image_count_1000_snapshot_and_hash() {
    assert_eq!(
        format!("{:x}", Sha256::digest(FIXTURE)),
        "bb2a895851de6af46b46951ad6e573513107ecad5432b324e0afb1c0281345d7"
    );
    let fixture: serde_json::Value = serde_json::from_slice(FIXTURE).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    assert_eq!(fixture["outline_preview"]["imageCount"], 2);
    assert!(fixture["outline_preview"].get("images").is_none());
}

#[test]
fn mp10_a_stored_tool_call_with_pictures_has_their_count_in_the_outline_preview() {
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry, SessionHistoryEntryKind};
    let fixture: serde_json::Value = serde_json::from_slice(FIXTURE).unwrap();
    let harness = LocalRouterTestHarness::new();
    let request = harness.fixture_session_request("outline-tool-images");
    let LocalDaemonResponse::SessionCreated { session, .. } = harness
        .dispatch(LocalDaemonRequest::CreateSession(request))
        .unwrap()
    else {
        panic!("session")
    };
    let agent_id = session.agents()[0].id().to_string();
    let store = harness.with_app(|app| app.operational_history_store());
    let context = HistoryEventTurnContext {
        session_id: Some(session.id().into()),
        agent_id: Some(agent_id.clone()),
        provider_run_id: Some("chart-run".into()),
        prompt_id: Some("prompt-chart".into()),
        turn_id: Some("turn-chart".into()),
        ..Default::default()
    };
    store
        .append_transcript(
            &SessionHistoryEntry::user_prompt(
                session.id(),
                "chart-client",
                &agent_id,
                "Draw the chart",
            ),
            context.clone(),
        )
        .unwrap();
    store
        .append_transcript(
            &SessionHistoryEntry::provider_output(
                session.id(),
                "chart-run",
                Some(&agent_id),
                TerminalOutputKind::ProviderTool,
                Some("call-chart".into()),
                fixture["stored_tool_call"].to_string(),
            ),
            context,
        )
        .unwrap();

    let LocalDaemonResponse::SessionHistoryOutline { agents } = harness
        .dispatch(LocalDaemonRequest::GetSessionHistoryOutline(
            crate::local::GetSessionHistoryOutlineRequest {
                session_id: session.id().into(),
                agent_ids: Some(vec![agent_id]),
                latest_prompt_count: Some(1),
                cursor: None,
                include_prefetched_content: false,
            },
        ))
        .unwrap()
    else {
        panic!("outline")
    };
    let tool = agents[0].turns[0]
        .blobs
        .iter()
        .find(|blob| blob.kind == SessionHistoryEntryKind::ProviderTool)
        .expect("tool blob");
    let preview: serde_json::Value =
        serde_json::from_str(tool.preview.as_deref().expect("tool preview")).unwrap();
    assert_eq!(preview, fixture["outline_preview"]);
}
