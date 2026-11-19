//! A turn Claude runs on its own, such as the report of a finished background
//! task, must not complete the prompt written behind it.
use super::*;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderOwnTurnSignal,
};

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
        notification_turn
            .iter()
            .chain(&prompt_turn)
            .all(|batch| batch.provider_own_turn == ProviderOwnTurnSignal::None),
        "a turn that carries a written prompt is not Claude's own"
    );
}

// What Claude Code 2.1.327 printed (r6c/sj-notif.jsonl) when a background command ended after
// the prompt's turn: `system/init` naming no user message opens Claude's own turn, and its
// `result` names none either. The fixture waits for a second line before that turn and a third
// before its end.
const CLAUDE_PROMPT_TURN_THEN_OWN_TURN: &str = r#"IFS= read -r prompt
uuid=$(printf '%s' "$prompt" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
printf '{"type":"system","subtype":"init","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
printf '%s\n' '{"type":"assistant","message":{"id":"msg_started","content":[{"type":"text","text":"STARTED"}]}}'
printf '{"type":"result","subtype":"success","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
IFS= read -r release
printf '%s\n' '{"type":"system","subtype":"task_notification","task_id":"bg1","tool_use_id":"toolu_bg","status":"completed","session_id":"s1"}'
printf '%s\n' '{"type":"system","subtype":"init","session_id":"s1"}'
printf '%s\n' '{"type":"assistant","message":{"id":"msg_own","content":[{"type":"text","text":"NOTIFIED"}]}}'
IFS= read -r finish
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
    assert!(prompt_turn
        .iter()
        .all(|batch| batch.provider_own_turn == ProviderOwnTurnSignal::None));
    assert!(binding.state.active_turn_id.is_none());

    write_json_line(&mut binding.state.stdin, &json!({"release": true})).unwrap();
    let own_turn = drain_until(&run, &mut binding.state, "the own turn's output", |batch| {
        prints(batch, "NOTIFIED")
    });
    assert_eq!(
        own_turn.last().unwrap().provider_own_turn,
        ProviderOwnTurnSignal::Running,
        "Claude's own turn was not reported as running"
    );
    assert!(!own_turn.iter().any(|batch| batch.prompt_completed));

    write_json_line(&mut binding.state.stdin, &json!({"finish": true})).unwrap();
    let own_turn_end = drain_until(&run, &mut binding.state, "the own turn's result", |batch| {
        batch.prompt_completed
    });
    assert_eq!(
        own_turn_end.last().unwrap().provider_own_turn,
        ProviderOwnTurnSignal::Ended,
        "Claude's own turn is not reported as ended by its result"
    );
    assert!(binding.state.active_turn_id.is_none());
}

// A background command can end right after the prompt's turn: Claude prints the prompt's
// `result` and its own turn's start back to back. One drain never hands both to the kernel: the
// prompt's turn ends in one batch, and the next batch starts Claude's own turn.
const CLAUDE_PROMPT_RESULT_AND_OWN_TURN_BACK_TO_BACK: &str = r#"IFS= read -r prompt
uuid=$(printf '%s' "$prompt" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
printf '{"type":"system","subtype":"init","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
printf '{"type":"result","subtype":"success","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
printf '%s\n' '{"type":"system","subtype":"init","session_id":"s1"}'
printf '%s\n' '{"type":"assistant","message":{"id":"msg_own","content":[{"type":"text","text":"NOTIFIED"}]}}'
printf '%s\n' '{"type":"assistant","message":{"id":"msg_mark","content":[{"type":"text","text":"PRINTED"}]}}'
IFS= read -r finish
"#;

#[test]
fn a_prompts_end_and_claudes_own_turn_start_never_share_a_batch() {
    let worktree = crate::test_support::TestWorktree::new("claude-own-turn-back-to-back");
    let run = RuntimeProviderRun::new(
        "own-turn-back-to-back-run",
        &LaunchProviderRequest::new("room", "claude", "claude", "default", "sonnet"),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-own-turn-back-to-back".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                CLAUDE_PROMPT_RESULT_AND_OWN_TURN_BACK_TO_BACK.into(),
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
    // Every line is printed before the first drain.
    std::thread::sleep(Duration::from_millis(300));
    let prompt_end = drain_claude_events(&run, &mut binding.state).unwrap();
    assert!(prompt_end.prompt_completed);
    assert!(
        prompt_end.provider_own_turn == ProviderOwnTurnSignal::None
            && !prints(&prompt_end, "NOTIFIED"),
        "the prompt's batch carries Claude's own turn"
    );
    let own_turn = drain_until(&run, &mut binding.state, "the own turn", |batch| {
        prints(batch, "PRINTED")
    });
    assert_eq!(
        own_turn.last().unwrap().provider_own_turn,
        ProviderOwnTurnSignal::Running
    );
    assert!(!own_turn.iter().any(|batch| batch.prompt_completed));
    write_json_line(&mut binding.state.stdin, &json!({"finish": true})).unwrap();
}

// T9: Claude's own turn can start and end before the kernel drains again; the batch still reports
// it, so the kernel adopts and completes it instead of leaving its output to no turn.
const CLAUDE_OWN_TURN_WITHIN_ONE_DRAIN: &str = r#"IFS= read -r prompt
uuid=$(printf '%s' "$prompt" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
printf '{"type":"system","subtype":"init","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
printf '{"type":"result","subtype":"success","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
IFS= read -r release
printf '%s\n' '{"type":"system","subtype":"init","session_id":"s1"}'
printf '%s\n' '{"type":"assistant","message":{"id":"msg_own","content":[{"type":"text","text":"NOTIFIED"}]}}'
printf '%s\n' '{"type":"result","subtype":"success","session_id":"s1"}'
IFS= read -r finish
"#;

#[test]
fn an_own_turn_that_starts_and_ends_within_one_drain_is_reported() {
    let worktree = crate::test_support::TestWorktree::new("claude-own-turn-one-drain");
    let run = RuntimeProviderRun::new(
        "own-turn-one-drain-run",
        &LaunchProviderRequest::new("room", "claude", "claude", "default", "sonnet"),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-own-turn-one-drain".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                CLAUDE_OWN_TURN_WITHIN_ONE_DRAIN.into(),
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
    drain_until(&run, &mut binding.state, "the prompt's result", |batch| {
        batch.prompt_completed
    });
    write_json_line(&mut binding.state.stdin, &json!({"release": true})).unwrap();
    // The whole own turn is printed before the next drain.
    std::thread::sleep(Duration::from_millis(300));
    let own_turn = drain_claude_events(&run, &mut binding.state).unwrap();
    assert!(prints(&own_turn, "NOTIFIED") && own_turn.prompt_completed);
    assert_eq!(
        own_turn.provider_own_turn,
        ProviderOwnTurnSignal::StartedAndEnded
    );
    write_json_line(&mut binding.state.stdin, &json!({"finish": true})).unwrap();
}

// A prompt arriving after the own-turn start must not clear its running signal.
#[test]
fn a_prompt_write_keeps_the_notification_turn_running_until_its_result() {
    let worktree = crate::test_support::TestWorktree::new("claude-write-during-own-turn");
    let run = RuntimeProviderRun::new(
        "write-during-own-turn-run",
        &LaunchProviderRequest::new("room", "claude", "claude", "default", "sonnet"),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-write-during-own-turn".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                (CLAUDE_PROMPT_TURN_THEN_OWN_TURN.to_string()
                    + r#"IFS= read -r release
uuid=$(printf '%s' "$finish" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
printf '%s\n' '{"type":"assistant","message":{"id":"msg_answer","content":[{"type":"text","text":"ANSWERED ONCE"}]}}'
printf '{"type":"result","subtype":"success","session_id":"s1","user_message_uuids":["%s"]}\n' "$uuid"
IFS= read -r end
"#),
                "fixture".into(),
            ],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    let mut binding = initialize_claude_runtime(&run).unwrap();
    let prompt = |text| {
        crate::prompt_assembly::PromptEnvelope::new(
            text,
            "",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::current(),
        )
    };
    submit_claude_prompt(&run, &mut binding.state, &prompt("start")).unwrap();
    drain_until(&run, &mut binding.state, "first result", |b| {
        b.prompt_completed
    });
    write_json_line(&mut binding.state.stdin, &json!({"release": true})).unwrap();
    drain_until(&run, &mut binding.state, "notification output", |b| {
        prints(b, "NOTIFIED")
    });
    submit_claude_prompt(&run, &mut binding.state, &prompt("status")).unwrap();
    assert!(
        binding.state.own_turn,
        "writing a prompt cannot end Claude's notification turn"
    );
    let batches = drain_until(&run, &mut binding.state, "notification result", |b| {
        b.provider_own_turn == ProviderOwnTurnSignal::Ended
    });
    assert!(
        !batches.iter().any(|b| b.prompt_completed),
        "the notification result does not answer the queued prompt"
    );
    assert!(
        binding.state.active_turn_id.is_some(),
        "the written prompt still awaits its result"
    );
    write_json_line(&mut binding.state.stdin, &json!({"release": true})).unwrap();
    let answer = drain_until(&run, &mut binding.state, "prompt result", |b| {
        b.prompt_completed
    });
    assert_eq!(
        answer.iter().filter(|b| prints(b, "ANSWERED ONCE")).count(),
        1
    );
    assert!(answer
        .iter()
        .all(|b| b.provider_own_turn == ProviderOwnTurnSignal::None));
    assert!(binding.state.active_turn_id.is_none());
}
