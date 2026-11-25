use super::*;
use crate::provider::claude_runtime::drain_claude_events;
use std::io::Write;

fn run() -> RuntimeProviderRun {
    RuntimeProviderRun::new(
        "run-1",
        &LaunchProviderRequest::new("session-1", "claude", "claude", "default", "sonnet"),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude:stream-json".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-claude-runtime".to_string()),
        },
    )
}

// MP-08 / MP-10: the structured Claude process runs the Chariox prompt hook; its
// receipt in the hook's event file is the only proof the context reached the model.
#[test]
fn context_delivered_receipt_in_the_hook_event_file_is_reported_once() {
    let dir = std::env::temp_dir().join(format!(
        "claude-hook-receipts-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let events = dir.join("events.jsonl");
    let (mut state, _) = parser_state();
    state.env.insert(
        "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
        events.display().to_string(),
    );
    let run = run();
    assert!(
        !drain_claude_events(&run, &mut state)
            .unwrap()
            .hook_context_delivered,
        "no event file yet"
    );
    let mut file = std::fs::File::create(&events).unwrap();
    writeln!(
        file,
        r#"{{"hook_event_name":"UserPromptSubmit","prompt":"$review"}}"#
    )
    .unwrap();
    assert!(
        !drain_claude_events(&run, &mut state)
            .unwrap()
            .hook_context_delivered,
        "the hook started but has not delivered"
    );
    write!(
        file,
        r#"{{"hook_event_name":"CharioxContextDelivered","hook_context_request_id":"r1"}}"#
    )
    .unwrap();
    assert!(
        !drain_claude_events(&run, &mut state)
            .unwrap()
            .hook_context_delivered,
        "a line the hook is still writing is not a receipt"
    );
    writeln!(file).unwrap();
    assert!(
        drain_claude_events(&run, &mut state)
            .unwrap()
            .hook_context_delivered
    );
    assert!(
        !drain_claude_events(&run, &mut state)
            .unwrap()
            .hook_context_delivered,
        "one receipt is reported once"
    );
    writeln!(
        file,
        r#"{{"hook_event_name":"PreToolUse","tool_name":"Bash"}}"#
    )
    .unwrap();
    assert!(
        !drain_claude_events(&run, &mut state)
            .unwrap()
            .hook_context_delivered
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn notification_hook_starts_the_provider_turn_without_waiting_for_stdout_init() {
    let (mut state, _) = parser_state();
    state.active_turn_id = None;
    let path =
        std::env::temp_dir().join(format!("notification-hook-{:032x}", rand::random::<u128>()));
    state.env.insert(
        "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
        path.display().to_string(),
    );
    let deadline = crate::session::unix_epoch_ms() + 5000;
    std::fs::write(
        &path,
        format!(
            "{}\n",
            json!({
                "hook_event_name": "UserPromptSubmit",
                "prompt": "<task-notification><task-id>bg1</task-id><tool-use-id>bg-call</tool-use-id><status>completed</status></task-notification>",
                "notification_context": true,
                "hook_context_request_id": "request-1",
                "hook_context_deadline_ms": deadline,
            })
        ),
    )
    .unwrap();

    let mut preceding = ProviderPromptSignalBatch {
        prompt_completed: true,
        ..Default::default()
    };
    crate::provider::claude_runtime::hook_receipts::read_prompt_hook_events(
        &mut state,
        &mut preceding,
    )
    .unwrap();
    assert!(preceding.claude_notification_context.is_none());
    assert!(state.active_turn_id.is_none());

    let run = run();
    let batch = drain_claude_events(&run, &mut state).unwrap();
    assert_eq!(
        batch.provider_own_turn,
        crate::provider::ProviderOwnTurnSignal::Running
    );
    let request = batch.claude_notification_context.unwrap();
    assert_eq!(request.request_id, "request-1");
    assert_eq!(request.ended_tool_use_id.as_deref(), Some("bg-call"));
    assert!(!batch.hook_context_delivered);
    assert!(drain_claude_events(&run, &mut state)
        .unwrap()
        .claude_notification_context
        .is_none());
    std::fs::remove_file(path).unwrap();
}
