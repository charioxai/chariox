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
        "local_protocol": 629,
