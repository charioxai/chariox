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
fn repeated_claude_permission_render_is_stored_once() {
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-permission-recent-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    let context_file = context_file.display().to_string();
    let rendered =
        "--dangerously-skip-permissions cannot be used with root/sudo privileges for security reasons";

    let first = update_claude_permission_recent(&context_file, rendered);
    let second = update_claude_permission_recent(&context_file, rendered);

    assert_eq!(first, second);
    assert_eq!(second.matches(rendered).count(), 1, "{second}");
    let _ = fs::remove_dir_all(root);
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
        "tool_name": "mcp__chariox__session_status",
    })));
    assert!(!should_bridge_claude_permission(&serde_json::json!({
        "hook_event_name": "PreToolUse",
        "permission_mode": "bypassPermissions",
        "tool_name": "Bash",
    })));
    assert!(!should_bridge_claude_permission(&serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "permission_mode": "bypassPermissions",
        "tool_name": "Bash",
    })));
}

#[test]
fn yolo_rendered_permission_is_confirmed_without_user_interaction() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-yolo-permission-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        "session-yolo",
        "claude",
        "claude-headless",
        "default",
        "claude-opus",
    )
    .with_agent_id("agent-yolo")
    .with_permission_level(crate::provider::AgentPermissionLevel::Yolo);
    let mut run = RuntimeProviderRun::new(
        "provider-run-yolo",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-yolo-permission".to_string(),
            pty_target: Some("test-claude-yolo-permission".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec!["-lc".to_string(), "cat".to_string()],
            pty_env: std::collections::BTreeMap::from([(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                context_file.clone(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    let bridge = RecordingPermissionBridge::default();
    let bridge_ref: std::sync::Arc<dyn ProviderNativeInteractionBridge> =
        std::sync::Arc::new(bridge.clone());

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            "session-yolo",
            run.id(),
            &run,
            Some(bridge_ref),
            "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("yolo permission should be confirmed");
    let confirmation_marker = root.join("yolo-rendered-permission-confirmed");
    let first_confirmation = fs::read_to_string(&confirmation_marker)
        .expect("yolo confirmation should leave a suppression marker");
    std::thread::sleep(std::time::Duration::from_millis(5));
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            "session-yolo",
            run.id(),
            &run,
            Some(std::sync::Arc::new(bridge.clone())),
            "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("lingering yolo permission should be suppressed");
    assert_eq!(
        fs::read_to_string(&confirmation_marker)
            .expect("yolo confirmation marker should remain present"),
        first_confirmation,
        "the same rendered prompt must not be confirmed twice"
    );
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            "session-yolo",
            run.id(),
            &run,
            Some(std::sync::Arc::new(bridge.clone())),
            "Write command\nprintf distinct\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("a distinct yolo permission should be confirmed immediately");
    assert_ne!(
        fs::read_to_string(&confirmation_marker)
            .expect("distinct yolo confirmation marker should be present"),
        first_confirmation,
        "suppression must be scoped to the exact rendered prompt"
    );
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            "session-yolo",
            run.id(),
            &run,
            Some(std::sync::Arc::new(bridge.clone())),
            "Claude composer ready",
        )
        .expect("dismissed permission should clear suppression state");
    assert!(
        !confirmation_marker.exists(),
        "a later permission prompt must be eligible for confirmation"
    );

    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        bridge
            .interaction_ids
            .lock()
            .expect("permission interaction recorder should not be poisoned")
            .is_empty(),
        "yolo permission must not create a user interaction"
    );
    assert!(!claude_native_marker(&context_file)
        .as_deref()
        .is_some_and(|marker| marker.starts_with("permission:")));
    app.pty
        .remove_process(run.id())
        .expect("test provider PTY should stop");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_permission_suppresses_post_stop_stale_rendered_permission_fallback() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-permission-dedupe-test-{}-{}",
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
    .with_agent_id("agent-1")
    .with_permission_level(crate::provider::AgentPermissionLevel::Required);
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
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
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
        "chariox-claude-permission-tombstone-test-{}-{}",
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
        "chariox-claude-permission-input-test-{}-{}",
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
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
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
fn headless_stop_stays_active_until_deferred_transcript_drain_finishes() {
    assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes(
        "claude-headless",
        false,
    );
}

#[test]
fn native_stop_stays_active_until_deferred_semantic_transcript_drain_finishes() {
    assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes("claude", false);
}

#[test]
fn late_claude_transcript_drain_does_not_complete_the_next_prompt() {
    assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes(
        "claude-headless",
        true,
    );
}

fn assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes(
    provider: &str,
    advance_next_prompt_before_late_drain: bool,
) {
    use std::io::Write as _;

    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-claude-headless-stop",
            "worktree-claude-headless-stop",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-claude-headless-stop",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-stop-test-{}-{}-{}",
        provider,
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    let transcript_file = root.join("session.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    let transcript_record = serde_json::json!({
        "type": "assistant",
        "uuid": "assistant-late",
        "message": {
            "id": "message-late",
            "role": "assistant",
            "content": [{ "type": "text", "text": "late final response" }]
        }
    })
    .to_string();
    let transcript_split_at = transcript_record.len() / 2;
    fs::write(&transcript_file, &transcript_record[..transcript_split_at])
        .expect("partial transcript output should be written");
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "Stop",
            "transcript_path": transcript_file.display().to_string(),
        })
        .to_string(),
    )
    .expect("Stop event should be written");
    let context_file = context_file.display().to_string();

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        provider,
        "default",
        "claude-sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-claude-headless-stop",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-stop".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
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
            "headless prompt",
            Vec::new(),
        )
        .expect("native prompt should be recorded")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    write_claude_native_marker(&context_file, &format!("injected:{}", prompt.id()));
    let queued_prompt = if advance_next_prompt_before_late_drain {
        match app
            .record_native_prompt_started_with_attachments(
                session.id(),
                attachment.id(),
                attachment.id(),
                agent.id(),
                "next headless prompt",
                Vec::new(),
            )
            .expect("next native prompt should be queued")
        {
            crate::session::PromptSubmissionOutcome::Queued { prompt } => Some(prompt),
            other => panic!("unexpected next prompt outcome: {other:?}"),
        }
    } else {
        None
    };

    let outcome = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("Stop event should be processed");

    assert!(outcome.needs_deferred_transcript_drain);
    assert!(app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should load")
        .is_some());
    assert!(claude_native_marker(&context_file)
        .as_deref()
        .is_some_and(|marker| marker.starts_with(CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX)));

    let next_active_prompt_id = if let Some(queued_prompt) = queued_prompt {
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .expect("stopped prompt should complete");
        let next_prompt_id = app.sessions_mut().reserve_prompt_id();
        let next = app
            .prompt_owner_activate_next_queued_prompt_with_prompt_id(
                session.id(),
                agent.id(),
                Some(queued_prompt.id()),
                next_prompt_id,
            )
            .expect("next prompt activation should succeed")
            .expect("next prompt should activate");
        crate::transport::flow_control::note_prompt_started(&mut app, run.id());
        Some(next.id().to_string())
    } else {
        None
    };

    std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript_file)
        .expect("partial transcript should reopen")
        .write_all(transcript_record[transcript_split_at..].as_bytes())
        .expect("late transcript output should finish writing");

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .drain_known_claude_transcripts(session.id(), run.id(), &context_file)
        .expect("late transcript output should drain");
    assert!(
        !crate::transport::flow_control::prompt_completion_recorded(&app, run.id()),
        "Claude transcript completion must not compete with the prompt-bound Stop settlement",
    );

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .finish_deferred_stop(session.id(), run.id(), &run)
        .expect("deferred transcript drain should finish");

    let active_prompt = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("settled prompt state should load");
    if let Some(next_active_prompt_id) = next_active_prompt_id {
        assert_eq!(
            active_prompt.as_ref().map(|prompt| prompt.id()),
            Some(next_active_prompt_id.as_str()),
            "the stale Stop and transcript flush must not settle the next prompt",
        );
    } else {
        assert!(active_prompt.is_none());
    }
    let output = app
        .terminal()
        .output_records()
        .into_iter()
        .flat_map(|record| record.bytes)
        .collect::<Vec<_>>();
    assert!(String::from_utf8_lossy(&output).contains("late final response"));

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
fn native_interrupt_control_prompt_is_not_user_history() {
    assert!(claude_native_prompt_is_internal_control(
        "[Request interrupted by user]"
    ));
    assert!(claude_native_prompt_is_internal_control(
        "  [Request interrupted by user]\n"
    ));
    assert!(!claude_native_prompt_is_internal_control(
        "Please explain why the request was interrupted by the user."
    ));
    assert!(claude_native_prompt_is_internal_control(
        "<task-notification><task-id>build-1</task-id><status>completed</status></task-notification>"
    ));
    assert!(claude_native_prompt_is_internal_control(
        "<task-notification><task-id>build-1</task-id></task-notification>\n\
         <task-notification><task-id>test-1</task-id></task-notification>"
    ));
    assert!(!claude_native_prompt_is_internal_control(
        "Review this <task-notification> example for correctness."
    ));
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
fn claude_headless_dispatch_waits_for_user_prompt_submit_acknowledgement() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-ack-test-{}-{}",
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
        "session-1",
        "claude",
        "claude-headless",
        "default",
        "claude-sonnet",
    )
    .with_agent_id("agent-1")
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-ack".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let prompt_id = "prompt-ack";
    write_claude_native_marker(&context_file, &format!("injected:{prompt_id}"));
    write_claude_headless_submit_retry(
        &context_file,
        prompt_id,
        0,
        unix_epoch_ms(),
        "Explain the lifecycle briefly.",
    );
    let dispatch = KernelPromptDispatch {
        session_id: "session-1".to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: "agent-1".to_string(),
        prompt_id: prompt_id.to_string(),
        target_active_prompt_id: None,
        source_attachment_id: "attachment-1".to_string(),
        prompt: "Explain the lifecycle briefly.".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    };

    let injected = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt("session-1", run.id(), &run, &dispatch)
        .expect("injected prompt should remain pending");
    assert_eq!(injected, ClaudeNativeDispatchAttempt::AwaitingInjection);

    fs::write(
        &events_file,
        [
            serde_json::json!({
                "hook_event_name": "Stop",
            })
            .to_string(),
            serde_json::json!({
                "hook_event_name": "UserPromptSubmit",
                "prompt": "Explain the lifecycle briefly.",
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("hook events should be written");
    let accepted = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt("session-1", run.id(), &run, &dispatch)
        .expect("hook-acknowledged prompt should complete dispatch");
    assert_eq!(accepted, ClaudeNativeDispatchAttempt::Completed);
    assert!(
        fs::read_to_string(&events_file)
            .expect("events should remain available to the normal output bridge")
            .contains("Stop"),
        "dispatch acknowledgement must not consume later lifecycle events"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_headless_steering_dispatch_waits_for_provider_acknowledgement() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-steering-injection-test-{}-{}",
        std::process::id(),
        unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("temporary directory should exist");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should exist");
    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude-headless",
        "default",
        "claude-opus-4-7",
    )
    .with_agent_id("agent-1");
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-steering-injection".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                context_file.to_string_lossy().into_owned(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let dispatch = KernelPromptDispatch {
        session_id: "session-1".to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: "agent-1".to_string(),
        prompt_id: "pending-steering-1".to_string(),
        target_active_prompt_id: Some("prompt-1".to_string()),
        source_attachment_id: "attachment-1".to_string(),
        prompt: "Add one sentence before finishing.".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: true,
    };

    write_claude_native_marker(
        &context_file.to_string_lossy(),
        "injected:pending-steering-1",
    );
    let injected = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt("session-1", run.id(), &run, &dispatch)
        .expect("injected steering should remain pending");

    assert_eq!(injected, ClaudeNativeDispatchAttempt::AwaitingInjection);

    write_claude_native_marker(
        &context_file.to_string_lossy(),
        "accepted:pending-steering-1",
    );
    let accepted = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt("session-1", run.id(), &run, &dispatch)
        .expect("acknowledged steering should complete dispatch");

    assert_eq!(accepted, ClaudeNativeDispatchAttempt::Completed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_headless_user_prompt_submit_acknowledges_matching_managed_dispatches() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-submit-wait-ack",
            "worktree-submit-wait-ack",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-submit-wait-ack",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-submit-wait-ack-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "A stale prompt from the active turn.",
        })
        .to_string(),
    )
    .expect("UserPromptSubmit event should be written");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        "default",
        "claude-sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-submit-wait-ack",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-submit-wait-ack".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    let active_prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "Run the web tests and summarize them.",
            Vec::new(),
        )
        .expect("active prompt should start")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    let steering_prompt_id = "leased-steer:steer-1";
    let steering_marker = format!("submit-wait:{steering_prompt_id}:{}", unix_epoch_ms());
    write_claude_native_marker(&context_file, &steering_marker);
    write_claude_headless_submit_retry(
        &context_file,
        steering_prompt_id,
        0,
        unix_epoch_ms(),
        "Also report the package version.",
    );

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("stale UserPromptSubmit should be consumed");
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(steering_marker.as_str())
    );

    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "Also report the package version.",
        })
        .to_string(),
    )
    .expect("matching UserPromptSubmit event should be written");
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("active UserPromptSubmit should be acknowledged");

    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("accepted:{steering_prompt_id}").as_str())
    );
    let projected = app
        .sessions()
        .get_session(session.id())
        .expect("session should remain available");
    assert_eq!(
        projected
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.id()),
        Some(active_prompt.id())
    );
    assert!(projected
        .queued_prompts_for_agent(agent.id())
        .is_none_or(|queue| queue.is_empty()));

    app.prompt_owner_cancel_active_prompt_only(session.id(), agent.id())
        .expect("managed prompt should cancel before its late hook acknowledgement");
    write_claude_native_marker(&context_file, &format!("injected:{}", active_prompt.id()));
    write_claude_headless_submit_retry(
        &context_file,
        active_prompt.id(),
        0,
        unix_epoch_ms(),
        "Run the web tests and summarize them.",
    );
    let terminal_input_count = app.terminal().input_records().len();
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "Run the web tests and summarize them.",
        })
        .to_string(),
    )
    .expect("late matching UserPromptSubmit event should be written");
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("late managed UserPromptSubmit should be consumed");

    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("accepted:{}", active_prompt.id()).as_str())
    );
    assert_eq!(app.terminal().input_records().len(), terminal_input_count);
    assert!(app
        .sessions()
        .get_session(session.id())
        .expect("session should remain available")
        .active_prompt_for_agent(agent.id())
        .is_none());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_native_dispatch_marker_extracts_steering_identity() {
    assert_eq!(
        claude_native_dispatch_prompt_id("injected:steering-prompt-1"),
        Some("steering-prompt-1")
    );
    assert_eq!(
        claude_native_dispatch_prompt_id("submit-wait:steering-prompt-1:1234"),
        Some("steering-prompt-1")
    );
    assert_eq!(claude_native_dispatch_prompt_id("startup-wait:1234"), None);
}

#[test]
fn claude_headless_steering_enqueue_acknowledges_exact_injected_prompt() {
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-steering-enqueue-ack-test-{}-{}",
        std::process::id(),
        unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    let steering_prompt_id = "leased-steer:steer-1";
    let steering_prompt = "Also report the package version.";
    write_claude_native_marker(&context_file, &format!("injected:{steering_prompt_id}"));
    write_claude_headless_submit_retry(
        &context_file,
        steering_prompt_id,
        0,
        unix_epoch_ms(),
        steering_prompt,
    );

    acknowledge_claude_headless_steering_enqueue(
        &context_file,
        Some("active-prompt-1"),
        &["A different queued prompt.".to_string()],
    );
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("injected:{steering_prompt_id}").as_str())
    );

    acknowledge_claude_headless_steering_enqueue(
        &context_file,
        Some("active-prompt-1"),
        &[steering_prompt.to_string()],
    );
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("accepted:{steering_prompt_id}").as_str())
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_transcript_drain_maps_assistant_text_reasoning_and_tools() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-test-{}",
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
fn claude_transcript_drain_skips_content_before_active_prompt() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-active-prompt-cutoff-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        [
            serde_json::json!({
                "type": "assistant",
                "uuid": "old-assistant",
                "timestamp": "1970-01-01T00:00:01Z",
                "message": {
                    "id": "old-message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "old response" }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "assistant",
                "uuid": "current-assistant",
                "timestamp": "1970-01-01T00:00:03Z",
                "message": {
                    "id": "current-message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "current response" }]
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file_since(
        &transcript.display().to_string(),
        &mut cursor,
        Some(2_000),
    );

    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(drain.chunks[0].text, "current response");
    assert_eq!(drain.assistant_message_ids, vec!["current-message"]);
    let second = drain_claude_transcript_file_since(
        &transcript.display().to_string(),
        &mut cursor,
        Some(2_000),
    );
    assert!(second.chunks.is_empty());
    assert!(second.assistant_message_ids.is_empty());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_drain_ignores_internal_resume_pair_before_real_response() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-resume-pair-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        [
            serde_json::json!({
                "type": "user",
                "uuid": "synthetic-user",
                "isMeta": true,
                "sessionId": "claude-session-1",
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Continue from where you left off." }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "assistant",
                "uuid": "synthetic-assistant",
                "parentUuid": "synthetic-user",
                "sessionId": "claude-session-1",
                "message": {
                    "id": "synthetic-message",
                    "model": "<synthetic>",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "No response requested." }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "user",
                "uuid": "real-user",
                "sessionId": "claude-session-1",
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Explain authoritative completion." }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "assistant",
                "uuid": "real-assistant",
                "sessionId": "claude-session-1",
                "message": {
                    "id": "real-message",
                    "model": "claude-opus-4-7",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Only the real response settles the turn." }]
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.session_id.as_deref(), Some("claude-session-1"));
    assert_eq!(drain.model.as_deref(), Some("claude/claude-opus-4-7"));
    assert_eq!(drain.assistant_message_ids, vec!["real-message"]);
    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(
        drain.chunks[0].text,
        "Only the real response settles the turn."
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_drain_preserves_synthetic_api_errors() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-api-error-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        serde_json::json!({
            "type": "assistant",
            "uuid": "api-error-assistant",
            "isApiErrorMessage": true,
            "error": "authentication_failed",
            "sessionId": "claude-session-error",
            "message": {
                "id": "api-error-message",
                "model": "<synthetic>",
                "role": "assistant",
                "content": [{ "type": "text", "text": "Login expired · Please run /login" }]
            }
        })
        .to_string(),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.session_id.as_deref(), Some("claude-session-error"));
    assert_eq!(drain.model, None);
    assert_eq!(drain.assistant_message_ids, vec!["api-error-message"]);
    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(drain.chunks[0].text, "Login expired · Please run /login");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_drain_exposes_queue_enqueues_without_rendering_them() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-dedupe-test-{}",
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
            serde_json::json!({
                "type": "queue-operation",
                "operation": "enqueue",
                "content": "Also report the package version."
            })
            .to_string(),
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
    assert_eq!(
        drain.enqueued_prompts,
        vec!["Also report the package version."]
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_completion_is_unique_per_assistant_message_id() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-message-completion-dedupe-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        [
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-entry-1",
                "message": {
                    "id": "msg_shared",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "first projection" }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-entry-2",
                "message": {
                    "id": "msg_shared",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "second projection" }]
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.chunks.len(), 2);
    assert_eq!(drain.assistant_message_ids, vec!["msg_shared"]);

    let second = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);
    assert!(second.assistant_message_ids.is_empty());

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
    let prompt = "Use the native-claude-skill skill. Give the Chariox skill marker.";
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
fn claude_headless_composer_detects_current_cycle_footer_only_with_prompt_glyph() {
    assert!(claude_headless_composer_visible(
        "──────────────── ❯ ──────────────── ⏵⏵ mode (shift+tab to cycle)"
    ));
    assert!(!claude_headless_composer_visible(
        "The documentation says shift+tab to cycle through modes."
    ));
    assert!(!claude_headless_composer_visible(
        "❯ stale composer frame followed much later by reviewer prose: The documentation says shift+tab to cycle through modes."
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
        "chariox-claude-bypass-selection-test-{}-{}",
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
