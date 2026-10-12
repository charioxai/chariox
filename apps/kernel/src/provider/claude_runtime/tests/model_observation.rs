use super::{apply_claude_message, json, parser_state};

#[test]
fn synthetic_quota_message_does_not_become_the_selected_model() {
    let (mut state, mut batch) = parser_state();
    apply_claude_message(
        "run-1",
        &mut state,
        json!({
            "type": "assistant",
            "message": {
                "id": "quota-error",
                "model": "<synthetic>",
                "content": [{"type": "text", "text": "Weekly usage limit reached"}]
            }
        }),
        &mut batch,
    );
    assert!(batch.resolved_model.is_none());
    assert!(batch.resolved_model_source.is_none());
    assert!(batch
        .chunks
        .iter()
        .any(|chunk| String::from_utf8_lossy(&chunk.bytes).contains("Weekly usage limit")));
}

#[test]
fn synthetic_stream_start_preserves_the_real_system_model() {
    let (mut state, mut batch) = parser_state();
    apply_claude_message(
        "run-1",
        &mut state,
        json!({"type": "system", "model": "claude-haiku-5-5"}),
        &mut batch,
    );
    apply_claude_message(
        "run-1",
        &mut state,
        json!({"type": "stream_event", "event": {
            "type": "message_start", "message": {"model": "<synthetic>"}
        }}),
        &mut batch,
    );
    assert_eq!(
        batch.resolved_model.as_deref(),
        Some("claude/claude-haiku-5-5")
    );
    assert_eq!(batch.resolved_model_source, Some("claude.system"));
}

#[test]
fn placeholder_system_model_does_not_hide_a_later_real_model() {
    let (mut state, mut batch) = parser_state();
    apply_claude_message(
        "run-1",
        &mut state,
        json!({"type": "system", "model": "claude/<synthetic>"}),
        &mut batch,
    );
    apply_claude_message(
        "run-1",
        &mut state,
        json!({"type": "assistant", "message": {"model": "custom-deployment"}}),
        &mut batch,
    );
    assert_eq!(
        batch.resolved_model.as_deref(),
        Some("claude/custom-deployment")
    );
    assert_eq!(batch.resolved_model_source, Some("claude.assistant"));
}
