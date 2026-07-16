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
fn claude_permission_detection_bridges_non_runtime_mcp_tools() {
    let mcp_permission = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-mcp",
        "tool_name": "mcp__native_test__echo_marker",
        "tool_input": { "marker": "native-skill-marker" },
    });
    assert!(should_bridge_claude_permission(&mcp_permission));
    assert!(claude_rendered_permission_visible(
        "Tool use native-test - echo_marker(marker: native-skill-marker) (MCP)\n\
         Do you want to proceed?\n1. Yes\n2. Yes, and don't ask again\n3. No",
    ));

    assert!(!should_bridge_claude_permission(&serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "mcp__arroba__session_status",
    })));
    assert!(!should_bridge_claude_permission(&serde_json::json!({
        "hook_event_name": "PreToolUse",
        "permission_mode": "bypassPermissions",
        "tool_name": "Bash",
    })));
}

#[test]
fn hook_permission_suppresses_post_stop_stale_rendered_permission_fallback() {
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
    std::thread::sleep(std::time::Duration::from_millis(100));
    write_claude_native_marker(&context_file, "");
    output
        .process_terminal_output(
            "session-1",
            run.id(),
            &run,
            Some(bridge_ref),
            "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("post-Stop stale rendered permission should be ignored");

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
fn hook_permission_tombstone_only_consumes_matching_rendered_frame() {
    let root = std::env::temp_dir().join(format!(
        "arroba-claude-permission-tombstone-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    write_claude_hook_permission_tombstone(
        &context_file,
        &serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": "echo first" },
        }),
    );

    assert!(!take_matching_claude_hook_permission_tombstone(
        &context_file,
        "Bash command\necho second\nDo you want to proceed?\n1. Yes\n3. No",
    ));
    assert!(take_matching_claude_hook_permission_tombstone(
        &context_file,
        "Bash command\necho first\nDo you want to proceed?\n1. Yes\n3. No",
    ));
    assert!(!take_matching_claude_hook_permission_tombstone(
        &context_file,
        "Bash command\necho first\nDo you want to proceed?\n1. Yes\n3. No",
    ));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn rendered_permission_resolution_does_not_reinject_native_prompt() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-claude-permission",
            "worktree-claude-permission",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-claude-permission",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let root = std::env::temp_dir().join(format!(
        "arroba-claude-permission-input-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let context_file = context_file.display().to_string();

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "claude-sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-claude-permission",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-native-permission".to_string(),
            pty_target: Some("test-claude-native-permission".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec!["-lc".to_string(), "cat".to_string()],
            pty_env: std::collections::BTreeMap::from([
                (
                    "ARROBA_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "ARROBA_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    let prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "native permission prompt",
            Vec::new(),
        )
        .expect("native prompt should be recorded")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    write_claude_native_marker(&context_file, "permission:interaction-1");
    write_claude_permission_input(&context_file, "interaction-1", b"1\r");

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("permission input should be processed");

    let marker = claude_native_marker(&context_file);
    app.pty
        .remove_process(run.id())
        .expect("test provider PTY should stop");
    let _ = fs::remove_dir_all(root);
    assert_eq!(
        marker.as_deref(),
        Some(format!("permission-resolved:{}", prompt.id()).as_str()),
        "permission approval must keep the active prompt marked as already injected",
    );
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

#[test]
fn claude_headless_bypass_confirmation_detects_clipped_rendered_choice() {
    let rendered = "WARNING:Claude CoderunninginBypassPermissionsmode \
        Byproceeding,youacceptallresponsibilityforactionstaken \
        >1.No,exit 2. Yes, I accep Entertoconfirm-Esc to cancel";

    assert!(claude_headless_bypass_confirmation_visible(rendered));
    assert!(!claude_headless_bypass_confirmation_visible(
        "Bypass permissions on - for shortcuts",
    ));
}

#[test]
fn claude_headless_bypass_selection_marker_is_distinct_from_prompt_state() {
    let root = std::env::temp_dir().join(format!(
        "arroba-claude-bypass-selection-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();

    assert!(!claude_headless_bypass_selection_pending(&context_file));
    write_claude_headless_bypass_selection_marker(&context_file);
    assert!(claude_headless_bypass_selection_pending(&context_file));
    write_claude_headless_startup_wait_marker(&context_file);
    assert!(!claude_headless_bypass_selection_pending(&context_file));

    let _ = fs::remove_dir_all(root);
}
