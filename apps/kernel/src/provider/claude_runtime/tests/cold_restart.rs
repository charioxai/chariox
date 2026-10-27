//! MP-08 / MP-10 / MP-11: an unused Claude session ID is not a persisted conversation.
use super::*;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderResumeState,
};

fn restarted_session_arguments(existing: Option<&str>, submitted: bool) -> (String, String) {
    let worktree = crate::test_support::TestWorktree::new("claude-cold-restart");
    let trace = worktree.path().join("argv.log");
    let mut request = LaunchProviderRequest::new("room", "claude", "claude", "default", "sonnet");
    if let Some(session) = existing {
        request = request.with_resume_state(ProviderResumeState::from_claude_session_id(session));
    }
    let run = RuntimeProviderRun::new(
        "cold-restart-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-argv-regression".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                r#"printf 'argv' >> "$CLAUDE_TEST_TRACE"
for arg in "$@"; do printf '\t%s' "$arg" >> "$CLAUDE_TEST_TRACE"; done
printf '\n' >> "$CLAUDE_TEST_TRACE"
session_id=''; previous=''
for arg in "$@"; do
  case "$previous" in --session-id|--resume) session_id="$arg" ;; esac
  previous="$arg"
done
while IFS= read -r line; do printf '{"type":"system","subtype":"init","session_id":"%s"}\n' "$session_id"; done"#
                    .into(),
                "fixture".into(),
            ],
            pty_env: BTreeMap::from([("CLAUDE_TEST_TRACE".into(), trace.display().to_string())]),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    let mut binding = initialize_claude_runtime(&run).unwrap();
    let session = binding
        .state
        .session_id()
        .map(str::to_string)
        .unwrap_or_else(|| {
            binding
                .state
                .args
                .windows(2)
                .find(|pair| pair[0] == "--session-id")
                .unwrap()[1]
                .clone()
        });
    wait_for_launches(&trace, 1);
    if submitted {
        let envelope = crate::prompt_assembly::PromptEnvelope::new(
            "real turn",
            "",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::current(),
        );
        submit_claude_prompt(&run, &mut binding.state, &envelope).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while binding.state.session_id().is_none() {
            super::drain_claude_events(&run, &mut binding.state).unwrap();
            assert!(
                Instant::now() < deadline,
                "official init must confirm a resumable session"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    restart_claude_runtime(&run, &mut binding.state, "regression_restart_selection").unwrap();
    let contents = wait_for_launches(&trace, 2);
    (contents.lines().last().unwrap().to_string(), session)
}

fn wait_for_launches(path: &std::path::Path, count: usize) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        if contents.ends_with('\n') && contents.lines().count() >= count {
            return contents;
        }
        assert!(Instant::now() < deadline, "child launch trace missing");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn selection_restart_before_first_prompt_keeps_a_new_session() {
    let (args, session) = restarted_session_arguments(None, false);
    assert!(
        args.ends_with(&format!("\t--session-id\t{session}")),
        "an unused session must start with --session-id, never --resume: {args}"
    );
    assert!(!args.contains("\t--resume\t"));
}

#[test]
fn first_selection_restart_can_resume_an_existing_conversation() {
    let (args, session) = restarted_session_arguments(Some("persisted-conversation"), false);
    assert!(args.ends_with(&format!("\t--resume\t{session}")));
}

#[test]
fn selection_restart_after_prompt_submission_preserves_the_session() {
    let (args, session) = restarted_session_arguments(None, true);
    assert!(args.ends_with(&format!("\t--resume\t{session}")));
}
