//! A turn Claude runs on its own, such as the report of a finished background
//! task, must not complete the prompt written behind it.
use super::*;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

// The order Claude Code 2.1.296 printed when a prompt was written while the
// notification turn of a finished background task was still running: the
// notification turn's `result` names no user message, then the prompt's own
// turn runs and its `result` names the prompt's `uuid`. The fixture holds the
// prompt's turn back until it reads a second line, and prints one frame after
// the notification `result` so the test knows that it was read.
const CLAUDE_NOTIFICATION_TURN_THEN_PROMPT_TURN: &str = r#"IFS= read -r prompt
uuid=$(printf '%s' "$prompt" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
printf '%s\n' '{"type":"system","subtype":"task_notification","task_id":"bg1","tool_use_id":"toolu_bg","status":"completed","session_id":"s1"}'
printf '%s\n' '{"type":"system","subtype":"init","session_id":"s1"}'
printf '%s\n' '{"type":"assistant","message":{"id":"msg_notice","content":[{"type":"text","text":"background task reported"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","session_id":"s1"}'
printf '%s\n' '{"type":"assistant","message":{"id":"msg_marker","content":[{"type":"text","text":"notification turn over"}]}}'
IFS= read -r release
printf '%s\n' '{"type":"assistant","message":{"id":"msg_answer","content":[{"type":"text","text":"answer to the prompt"}]}}'
printf '{"type":"result","subtype":"success","session_id":"s1","user_message_uuid":"%s","user_message_uuids":["%s"]}\n' "$uuid" "$uuid"
"#;

/// Drains until `done` holds for a batch; every batch drained so far is returned.
fn drain_until(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    what: &str,
    done: impl Fn(&crate::provider::ProviderPromptSignalBatch) -> bool,
) -> Vec<crate::provider::ProviderPromptSignalBatch> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut batches = Vec::new();
    loop {
        let batch = drain_claude_events(run, state).unwrap();
        let finished = done(&batch);
        batches.push(batch);
        if finished {
            return batches;
        }
        assert!(Instant::now() < deadline, "never saw {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn prints(batch: &crate::provider::ProviderPromptSignalBatch, text: &str) -> bool {
    batch
        .chunks
        .iter()
        .any(|chunk| String::from_utf8_lossy(&chunk.bytes).contains(text))
}

#[test]
fn a_notification_turn_result_does_not_complete_the_prompt_written_behind_it() {
    let worktree = crate::test_support::TestWorktree::new("claude-notification-turn");
    let run = RuntimeProviderRun::new(
        "notification-turn-run",
        &LaunchProviderRequest::new("room", "claude", "claude", "default", "sonnet"),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-notification-turn".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                CLAUDE_NOTIFICATION_TURN_THEN_PROMPT_TURN.into(),
                "fixture".into(),
            ],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    let mut binding = initialize_claude_runtime(&run).unwrap();
    let envelope = crate::prompt_assembly::PromptEnvelope::new(
        "next prompt",
        "",
        Vec::new(),
        crate::prompt_assembly::PromptManifest::current(),
    );
    submit_claude_prompt(&run, &mut binding.state, &envelope).unwrap();

    let notification_turn = drain_until(
        &run,
        &mut binding.state,
        "the end of the notification turn",
        |batch| prints(batch, "notification turn over"),
    );
    assert!(
        !notification_turn.iter().any(|batch| batch.prompt_completed),
        "the notification turn's result completed the prompt written behind it"
    );
    assert!(
        binding.state.active_turn_id.is_some(),
        "the prompt's own turn has not run yet"
    );

    write_json_line(&mut binding.state.stdin, &json!({"release": true})).unwrap();
    let prompt_turn = drain_until(
        &run,
        &mut binding.state,
        "the result of the prompt's turn",
        |batch| batch.prompt_completed,
    );
    assert!(prompt_turn
        .iter()
        .any(|batch| prints(batch, "answer to the prompt")));
    assert!(binding.state.active_turn_id.is_none());
    assert!(
        !notification_turn
            .iter()
            .chain(&prompt_turn)
            .any(|batch| batch.provider_turn_started),
        "a turn that carries a written prompt is not Claude's own"
    );
}

// What Claude Code 2.1.327 printed (r6c/sj-notif.jsonl) when a background command ended after
// the prompt's turn: `system/init` naming no user message opens Claude's own turn, and its
// `result` names none either. The fixture waits for a second line before that turn.
const CLAUDE_PROMPT_TURN_THEN_OWN_TURN: &str = r#"IFS= read -r prompt
uuid=$(printf '%s' "$prompt" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
printf '{"type":"system","subtype":"init","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
printf '%s\n' '{"type":"assistant","message":{"id":"msg_started","content":[{"type":"text","text":"STARTED"}]}}'
printf '{"type":"result","subtype":"success","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
IFS= read -r release
printf '%s\n' '{"type":"system","subtype":"task_notification","task_id":"bg1","tool_use_id":"toolu_bg","status":"completed","session_id":"s1"}'
printf '%s\n' '{"type":"system","subtype":"init","session_id":"s1"}'
printf '%s\n' '{"type":"assistant","message":{"id":"msg_own","content":[{"type":"text","text":"NOTIFIED"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","session_id":"s1"}'
"#;

#[test]
fn claudes_own_turn_after_an_idle_prompt_turn_is_reported_from_start_to_end() {
    let worktree = crate::test_support::TestWorktree::new("claude-own-turn");
    let run = RuntimeProviderRun::new(
        "own-turn-run",
        &LaunchProviderRequest::new("room", "claude", "claude", "default", "sonnet"),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-own-turn".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                CLAUDE_PROMPT_TURN_THEN_OWN_TURN.into(),
                "fixture".into(),
            ],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    let mut binding = initialize_claude_runtime(&run).unwrap();
    let envelope = crate::prompt_assembly::PromptEnvelope::new(
        "start the background command",
        "",
        Vec::new(),
        crate::prompt_assembly::PromptManifest::current(),
    );
    submit_claude_prompt(&run, &mut binding.state, &envelope).unwrap();
    let prompt_turn = drain_until(&run, &mut binding.state, "the prompt's result", |batch| {
        batch.prompt_completed
    });
    assert!(!prompt_turn.iter().any(|batch| batch.provider_turn_started));
    assert!(binding.state.active_turn_id.is_none());

    write_json_line(&mut binding.state.stdin, &json!({"release": true})).unwrap();
    let own_turn = drain_until(&run, &mut binding.state, "the own turn's result", |batch| {
        batch.prompt_completed
    });
    assert!(
        own_turn.iter().any(|batch| batch.provider_turn_started),
        "Claude's own turn was not reported as started"
    );
    assert!(own_turn.iter().any(|batch| prints(batch, "NOTIFIED")));
    assert!(binding.state.active_turn_id.is_none());
}
