//! MP-08 / MP-10 / MP-11: protocol 596 events and subscribe field that spare a client its re-reads.
use super::*;
use crate::transport::kernel_protocol::{KernelEvent, KernelIncomingFrame};

fn turn() -> crate::local::SessionHistoryOutlineTurn {
    let user_prompt = history_page_entry(
        10,
        crate::history::SessionHistoryEntryKind::UserPrompt,
        "agent-1",
        "list the files",
    );
    let assistant = history_page_entry(
        12,
        crate::history::SessionHistoryEntryKind::ProviderOutput,
        "agent-1",
        "Two files.",
    );
    crate::local::SessionHistoryOutlineTurn {
        turn_id: "prompt-1".into(),
        prompt_id: Some("prompt-1".into()),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        started_at_ms: 42,
        lifecycle: crate::local::SessionHistoryOutlineTurnLifecycle::Completed,
        completed_at_ms: Some(99),
        outcome: None,
        user_prompt,
        entries: vec![assistant],
        summary: None,
        blobs: vec![crate::local::SessionHistoryOutlineBlob {
            blob_id: "history:11:11".into(),
            steering: false,
            kind: crate::history::SessionHistoryEntryKind::ProviderTool,
            title: "bash · COMPLETED".into(),
            summary: "$ ls".into(),
            sequence_start: 11,
            sequence_end: 11,
            entry_count: 1,
            total_chars: 20,
            timestamp_ms: 43,
            preview: None,
        }],
    }
}

fn fixture_events() -> serde_json::Value {
    let turn_changed = KernelEvent::AgentTurnChanged {
        session_id: "session-1".into(),
        agent_id: "agent-1".into(),
        history_revision: 12,
        turn: Box::new(turn()),
    };
    let audit_appended = KernelEvent::HomeExtensionAuditAppended {
        session_id: "session-1".into(),
        agent_id: "agent-1".into(),
        audit_event: Box::new(crate::durable_state::DurableStateEvent {
            sequence: 7,
            event_id: "state_evt_1".into(),
            kind: "home_extension.grant.created".into(),
            subject_id: Some("agent-1".into()),
            timestamp_ms: 50,
            payload: serde_json::json!({"name": "tool-1"}),
        }),
    };
    let subscribe = KernelIncomingFrame::Subscribe {
        request_id: "request-1".into(),
        session_id: "__waiting_room_inventory__".into(),
        attachment_id: "__waiting_room_inventory__".into(),
        subscription_scope: Some("waiting_room_inventory".into()),
        resume_from_event_id: None,
        heartbeat_interval_ms: Some(1_000),
        kernel: None,
    };
    serde_json::json!({
        "items": ["MP-08", "MP-10", "MP-11"],
        "local_protocol": LOCAL_DAEMON_PROTOCOL_VERSION,
        "allocation": "614/220 union (feature predecessor 596/202): agent_turn_changed, home_extension_audit_appended, home_extension_audit_lagged, heartbeat_interval_ms",
        "agent_turn_changed": serde_json::to_value(turn_changed).unwrap(),
        "home_extension_audit_appended": serde_json::to_value(audit_appended).unwrap(),
        "home_extension_audit_lagged": serde_json::to_value(KernelEvent::HomeExtensionAuditLagged {
            session_id: "session-1".into(),
        }).unwrap(),
        "subscribe_with_heartbeat_interval": serde_json::to_value(subscribe).unwrap(),
    })
}

#[test]
fn session_feed_596_shapes_are_pinned() {
    let bytes = include_bytes!("session-feed-596.json");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(bytes)),
        "edd903eaf81368f162706ace5f37fd8c297cdf72e9dd891f7888245da7852eb0"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture, fixture_events());
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    // The turn is the history outline's turn, shape for shape.
    let turn: crate::local::SessionHistoryOutlineTurn =
        serde_json::from_value(fixture["agent_turn_changed"]["turn"].clone()).unwrap();
    assert_eq!(turn, self::turn());
    // The audit row is the row `ListHomeExtensionAudit` returns; the event name is the tag, so the
    // row travels as `audit_event`.
    let audit: KernelEvent =
        serde_json::from_value(fixture["home_extension_audit_appended"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(audit).unwrap(),
        fixture["home_extension_audit_appended"]
    );
    assert_eq!(
        fixture["home_extension_audit_appended"]["event"],
        "home_extension_audit_appended"
    );
    // A subscribe that leaves the field out reads as the default interval.
    let without: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "subscribe", "request_id": "request-2", "session_id": "session-1",
        "attachment_id": "attachment-1",
    }))
    .unwrap();
    assert!(matches!(
        without,
        KernelIncomingFrame::Subscribe {
            heartbeat_interval_ms: None,
            ..
        }
    ));
}
