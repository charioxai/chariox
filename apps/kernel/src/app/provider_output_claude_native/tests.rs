use super::transcript::ClaudeTranscriptCursor;
use super::*;

#[derive(Clone, Default)]
struct RecordingPermissionBridge {
    interaction_ids: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl ProviderNativeInteractionBridge for RecordingPermissionBridge {
    fn request_blocking(
        &self,
        _session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<crate::provider::ProviderNativeInteractionResolution, DaemonError> {
        self.interaction_ids
            .lock()
            .expect("permission interaction recorder should not be poisoned")
            .push(interaction.id().to_string());
        Ok(crate::provider::ProviderNativeInteractionResolution {
            status: "answered".to_string(),
            choice_id: Some("allow_once".to_string()),
            reply: Some("allow".to_string()),
        })
    }
}

#[test]
fn hook_permission_suppresses_stale_rendered_permission_fallback() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "arroba-claude-permission-dedupe-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();

    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude",
        "default",
        "claude-sonnet",
    )
    .with_agent_id("agent-1");
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-native".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([(
                "ARROBA_CLAUDE_NATIVE_CONTEXT".to_string(),
                context_file.clone(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let bridge = RecordingPermissionBridge::default();
    let bridge_ref: std::sync::Arc<dyn ProviderNativeInteractionBridge> =
        std::sync::Arc::new(bridge.clone());
    let event = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-1",
        "tool_name": "Bash",
        "tool_input": { "command": "echo test" },
    });

    let mut output = ProviderOutputClaudeNativeBridge::new(&mut app);
    output
        .resolve_permission_event(
            "session-1",
            run.id(),
            "agent-1",
            &context_file,
            Some(bridge_ref.clone()),
            &event,
        )
        .expect("hook permission should be bridged");
    let attachments = Vec::new();
    output
        .inject_prompt(
            "session-1",
            run.id(),
            "agent-1",
            &context_file,
            &run,
            &ClaudeNativePromptInjection {
                id: "prompt-1",
                prompt: "do the work",
                hidden_system_context: "",
                attachments: &attachments,
            },
        )
        .expect("pending permission must not reinject the active prompt");
    output
        .process_terminal_output(
            "session-1",
            run.id(),
            &run,
            Some(bridge_ref),
            "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("stale rendered permission should be ignored");

    std::thread::sleep(std::time::Duration::from_millis(100));
    let interaction_ids = bridge
        .interaction_ids
        .lock()
        .expect("permission interaction recorder should not be poisoned")
        .clone();
    assert_eq!(
        interaction_ids,
        vec!["claude-native-permission-provider-run-1-request-1"],
        "one Claude permission must project exactly once across hook and rendered output"
    );

    let _ = fs::remove_dir_all(root);
}

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
        "paste again to expand - [Pasted text #2]",
        "expected prompt",
    ));
    assert!(claude_headless_prompt_waiting_in_composer(
        "\u{1b}[2m[Pasted text #1]\u{1b}[0m",
        "expected prompt",
    ));
    assert!(!claude_headless_prompt_waiting_in_composer(
        "CLAUDE-HEADLESS_WORKSPACE_LIVE_SYNC_TEXT_WRITE_DONE",
        "expected prompt",
    ));
}

#[test]
fn claude_headless_prompt_waiting_in_composer_detects_direct_prompt_text() {
    let prompt = "Use the native-claude-skill skill. Give the Arroba skill marker.";
    let rendered = format!("\u{1b}[?25l\u{1b}[H\r\u{1b}[37B{prompt}\u{1b}[40;1H\u{1b}[?25h");

    assert!(claude_headless_prompt_waiting_in_composer(
        &rendered, prompt
    ));
    assert!(!claude_headless_prompt_waiting_in_composer(
        &format!("{rendered} Gitifying... esc to interrupt"),
        prompt,
    ));
}
