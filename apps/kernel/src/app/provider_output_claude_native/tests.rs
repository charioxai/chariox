use super::transcript::ClaudeTranscriptCursor;
use super::*;

#[test]
fn native_prompt_history_uses_latest_terminal_input_attachment() {
    let app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    app.terminal().record_input(
        "session-1",
        "provider-run-1",
        "attachment-native-a",
        b"first",
    );
    app.terminal()
        .record_input("session-1", "provider-run-2", "attachment-other", b"other");
    app.terminal().record_input(
        "session-1",
        "provider-run-1",
        "attachment-native-b",
        b"second",
    );

    assert_eq!(
        claude_native_history_source_attachment_id(
            &app,
            "session-1",
            "provider-run-1",
            "attachment-fallback",
        ),
        "attachment-native-b"
    );
    assert_eq!(
        claude_native_history_source_attachment_id(
            &app,
            "session-1",
            "provider-run-missing",
            "attachment-fallback",
        ),
        "attachment-fallback"
    );
}

#[test]
fn submit_wait_state_defers_enter_until_delay_elapses() {
    let marker = format!("submit-wait:prompt-7:{}", 1_000);
    // Before the settle delay: keep waiting (Enter stays off the lock).
    assert_eq!(
        submit_wait_state(
            Some(&marker),
            "prompt-7",
            1_000 + CLAUDE_SUBMIT_DELAY_MS - 1
        ),
        SubmitWaitState::Waiting
    );
    // At/after the delay: submit the Enter keystroke.
    assert_eq!(
        submit_wait_state(Some(&marker), "prompt-7", 1_000 + CLAUDE_SUBMIT_DELAY_MS),
        SubmitWaitState::ReadyToSubmit
    );
    // A marker for a different prompt is not a submit-wait for this one.
    assert_eq!(
        submit_wait_state(Some(&marker), "prompt-8", 10_000),
        SubmitWaitState::NotSubmitWait
    );
    // Unrelated and malformed markers are ignored.
    assert_eq!(
        submit_wait_state(Some("injected:prompt-7"), "prompt-7", 10_000),
        SubmitWaitState::NotSubmitWait
    );
    // An unparseable timestamp submits rather than stalling forever.
    assert_eq!(
        submit_wait_state(Some("submit-wait:prompt-7:bogus"), "prompt-7", 10_000),
        SubmitWaitState::ReadyToSubmit
    );
}

#[test]
fn claude_transcript_drain_maps_assistant_text_reasoning_and_tools() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "arroba-claude-transcript-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
            &transcript,
            [
                serde_json::json!({
                    "type": "assistant",
                    "uuid": "assistant-1",
                    "sessionId": "claude-session-1",
                    "message": {
                        "id": "msg_1",
                        "model": "claude-sonnet-4-6",
                        "role": "assistant",
                        "content": [
                            { "type": "thinking", "thinking": "considering" },
                            { "type": "text", "text": "hello" },
                            { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "pwd" } }
                        ]
                    }
                })
                .to_string(),
                serde_json::json!({
                    "type": "user",
                    "uuid": "user-1",
                    "message": {
                        "role": "user",
                        "content": [
                            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" }
                        ]
                    }
                })
                .to_string(),
            ]
            .join("\n"),
        )
        .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.session_id.as_deref(), Some("claude-session-1"));
    assert_eq!(drain.model.as_deref(), Some("claude/claude-sonnet-4-6"));
    assert_eq!(drain.assistant_message_ids, vec!["msg_1"]);
    assert_eq!(drain.chunks.len(), 4);
    assert_eq!(drain.chunks[0].kind, TerminalOutputKind::ProviderReasoning);
    assert_eq!(drain.chunks[0].text, "considering");
    assert_eq!(drain.chunks[1].kind, TerminalOutputKind::ProviderOutput);
    assert_eq!(drain.chunks[1].text, "hello");
    assert_eq!(drain.chunks[2].kind, TerminalOutputKind::ProviderTool);
    assert!(drain.chunks[2].text.contains("\"tool\":\"Bash\""));
    assert!(drain.chunks[3].text.contains("\"status\":\"completed\""));

    let second = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);
    assert!(second.chunks.is_empty());
    assert!(second.assistant_message_ids.is_empty());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_drain_skips_internal_and_duplicate_entries() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "arroba-claude-transcript-dedupe-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    let assistant = serde_json::json!({
        "type": "assistant",
        "uuid": "assistant-1",
        "message": {
            "role": "assistant",
            "content": [{ "type": "text", "text": "once" }]
        }
    })
    .to_string();
    fs::write(
        &transcript,
        [
            serde_json::json!({ "type": "queue-operation", "operation": "enqueue" }).to_string(),
            assistant.clone(),
            assistant,
        ]
        .join("\n"),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(drain.chunks[0].text, "once");
    assert_eq!(drain.assistant_message_ids, vec!["assistant-1"]);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_headless_prompt_waiting_in_composer_detects_collapsed_paste() {
    assert!(claude_headless_prompt_waiting_in_composer(
        "paste again to expand - [Pasted text #2]"
    ));
    assert!(claude_headless_prompt_waiting_in_composer(
        "\u{1b}[2m[Pasted text #1]\u{1b}[0m"
    ));
    assert!(!claude_headless_prompt_waiting_in_composer(
        "CLAUDE-HEADLESS_WORKSPACE_LIVE_SYNC_TEXT_WRITE_DONE"
    ));
}
