//! MP-08 / MP-10 / MP-11: completed HTTP parts cannot be replayed by delayed SSE deltas.
use super::{
    parts::handle_message_part_delta, snapshot::render_snapshot_output_chunks, OpenCodeRuntimeState,
};
use crate::{
    extension::RemoteExtensionManifest,
    provider::{opencode_client::OpenCodeEventSubscription, OpenCodeMessage},
};

fn rendered_then_delta(kind: &str, completed: bool, metadata_only: bool) -> String {
    let mut state = OpenCodeRuntimeState::new(
        "http://localhost:1".into(),
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(std::sync::mpsc::channel().1),
    );
    state.note_prompt_submitted("user-1".into());
    let message: OpenCodeMessage = serde_json::from_value(serde_json::json!({
        "info": {"id":"message-1","sessionID":"session-1","parentID":"user-1","role":"assistant","time":{"completed": if completed {Some(1)} else {None}}},
        "parts": [{"id":"part-1","sessionID":"session-1","messageID":"message-1","type":kind,"text":"answer"}]
    })).unwrap();
    let snapshot =
        render_snapshot_output_chunks(&mut state, &RemoteExtensionManifest::default(), &[message]);
    if metadata_only {
        state
            .completed_assistant_message_ids
            .insert("message-1".into());
    }
    state
        .streamed_text_by_part
        .insert("part-1".into(), "answer".into());
    let mut chunks = snapshot.chunks;
    handle_message_part_delta(
        &mut state,
        "run-1",
        "session-1".into(),
        "message-1".into(),
        "part-1".into(),
        "text".into(),
        "answer".into(),
        &mut chunks,
    )
    .unwrap();
    chunks
        .into_iter()
        .map(|c| String::from_utf8(c.bytes).unwrap())
        .collect()
}

#[test]
fn completed_http_text_snapshot_fences_late_sse_delta() {
    assert_eq!(rendered_then_delta("text", true, false), "answer");
}
#[test]
fn completed_http_reasoning_snapshot_fences_late_sse_delta() {
    assert_eq!(rendered_then_delta("reasoning", true, false), "answer");
}
#[test]
fn unfinished_http_snapshot_preserves_repeated_text_delta() {
    assert_eq!(rendered_then_delta("text", false, false), "answeranswer");
}
#[test]
fn completion_metadata_without_completed_snapshot_preserves_delta() {
    assert_eq!(rendered_then_delta("text", false, true), "answeranswer");
}

#[test]
fn completed_empty_snapshot_does_not_fence_later_authoritative_text() {
    let mut state = OpenCodeRuntimeState::new(
        "http://localhost:1".into(),
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(std::sync::mpsc::channel().1),
    );
    state.note_prompt_submitted("user-1".into());
    let message: OpenCodeMessage = serde_json::from_value(serde_json::json!({
        "info": {"id":"message-1","sessionID":"session-1","parentID":"user-1","role":"assistant","time":{"completed":1}},
        "parts": [{"id":"part-1","sessionID":"session-1","messageID":"message-1","type":"text","text":""}]
    })).unwrap();
    let mut chunks =
        render_snapshot_output_chunks(&mut state, &RemoteExtensionManifest::default(), &[message])
            .chunks;
    handle_message_part_delta(
        &mut state,
        "run-1",
        "session-1".into(),
        "message-1".into(),
        "part-1".into(),
        "text".into(),
        "answer".into(),
        &mut chunks,
    )
    .unwrap();
    assert!(!state.finalized_text_part_ids.contains("part-1"));
    assert!(chunks.is_empty());
    super::parts::emit_authoritative_part_text(
        &mut state,
        "part-1",
        "answer",
        crate::terminal::TerminalOutputKind::ProviderOutput,
        &mut chunks,
    );
    assert_eq!(
        chunks
            .iter()
            .flat_map(|c| c.bytes.clone())
            .collect::<Vec<_>>(),
        b"answer"
    );
}
