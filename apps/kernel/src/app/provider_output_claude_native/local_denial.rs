//! MP-08 / MP-10 / MP-11: the person answers "No" in Claude's own permission
//! dialog. Claude runs no hook for that answer and writes no Stop: it records
//! the refusal as an interrupted tool use and returns to its prompt. Without
//! this settlement the kernel card stays open until its deadline and the
//! prompt stays running, so a prompt sent from another client never reaches
//! the terminal.
use super::permission::{
    claude_native_marker, write_claude_native_marker, write_claude_permission_response,
};
use super::CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX;
use crate::session::unix_epoch_ms;

/// Ends the turn of `prompt_id` the way a Stop would, when a kernel permission
/// card of this run is waiting. Returns whether a card was waiting.
pub(super) fn settle_denial_in_claude_dialog(
    context_file: &str,
    provider_run_id: &str,
    prompt_id: &str,
) -> bool {
    let Some(interaction_id) = claude_native_marker(context_file)
        .and_then(|marker| marker.strip_prefix("permission:").map(str::to_string))
    else {
        return false;
    };
    let hook_prefix = format!("claude-native-permission-{provider_run_id}-");
    if let Some(request_id) = interaction_id.strip_prefix(&hook_prefix) {
        write_claude_permission_response(
            context_file,
            request_id,
            false,
            "The permission was refused in Claude's own dialog.",
        );
    }
    write_claude_native_marker(
        context_file,
        &format!(
            "{CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX}{prompt_id}:{}",
            unix_epoch_ms()
        ),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_card_ends_with_the_turn_when_the_person_refuses_in_claude() {
        let root = crate::test_support::TestWorktree::new("claude-local-denial");
        let context = root.path().join("hidden-context.txt");
        let context_file = context.to_str().unwrap();
        write_claude_native_marker(
            context_file,
            "permission:claude-native-permission-run-1-request-9",
        );
        assert!(settle_denial_in_claude_dialog(
            context_file,
            "run-1",
            "prompt-7"
        ));
        let marker = claude_native_marker(context_file).unwrap();
        assert!(
            marker.starts_with("stop-draining:prompt-7:"),
            "the turn is settled like a Stop: {marker}"
        );
        let released = root.path().join("permission-responses/request-9.json");
        assert!(released.exists(), "the waiting hook is released");
    }

    #[test]
    fn claude_records_a_refusal_in_its_dialog_as_an_interrupted_tool_use() {
        let raw = concat!(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"The user doesn't want to proceed with this tool use."}]}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#,
            "\n",
        );
        let mut cursor = super::super::transcript::ClaudeTranscriptCursor::default();
        let drain = super::super::transcript::drain_claude_transcript_raw_since(
            "t.jsonl",
            raw,
            &mut cursor,
            None,
            1,
        );
        assert!(drain.user_denied_tool_use);
        let mut cursor = super::super::transcript::ClaudeTranscriptCursor::default();
        let plain = raw.lines().next().unwrap();
        let drain = super::super::transcript::drain_claude_transcript_raw_since(
            "t.jsonl",
            plain,
            &mut cursor,
            None,
            1,
        );
        assert!(!drain.user_denied_tool_use);
    }

    #[test]
    fn a_refusal_with_no_card_waiting_changes_nothing() {
        let root = crate::test_support::TestWorktree::new("claude-local-denial-none");
        let context = root.path().join("hidden-context.txt");
        let context_file = context.to_str().unwrap();
        write_claude_native_marker(context_file, "accepted:prompt-7");
        assert!(!settle_denial_in_claude_dialog(
            context_file,
            "run-1",
            "prompt-7"
        ));
        assert_eq!(
            claude_native_marker(context_file).as_deref(),
            Some("accepted:prompt-7")
        );
    }
}
