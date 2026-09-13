use super::*;
use crate::provider::claude_runtime::process::{write_json_line, ClaudeRuntimeMessage};
use std::time::Duration;

#[test]
#[ignore = "requires Bubblewrap and Linux user namespaces; run explicitly on the managed host"]
fn claude_sandbox_survives_launching_thread_exit() {
    let caller = std::thread::spawn(|| {
        let request =
            LaunchProviderRequest::new("lifetime-room", "claude", "claude", "default", "sonnet");
        let run = RuntimeProviderRun::new("claude-lifetime", &request, ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude:stream-json".into(),
            pty_target: None,
            pty_program: Some("/usr/bin/bwrap".into()),
            pty_args: ["--die-with-parent", "--new-session", "--unshare-user",
                "--unshare-pid", "--uid", "0", "--gid", "0", "--ro-bind", "/", "/",
                "--", "/bin/sh", "-c",
                "printf '{\"ready\":true}\\n'; while IFS= read -r line; do printf '%s\\n' \"$line\"; done", "fixture"]
                .into_iter().map(str::to_string).collect(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        });
        let binding = initialize_claude_runtime(&run).expect("sandbox should launch");
        let ready = binding.state.receiver.recv_timeout(Duration::from_secs(5));
        assert!(
            matches!(ready, Ok(ClaudeRuntimeMessage::Stdout(value)) if value == json!({"ready": true})),
            "sandbox must be ready before retiring its launching thread"
        );
        binding
    });
    let mut binding = caller.join().expect("launching thread should return");
    std::thread::sleep(Duration::from_millis(250));
    let write = write_json_line(
        &mut binding.state.stdin,
        &json!({"ping": "after-thread-exit"}),
    );
    let response = binding.state.receiver.recv_timeout(Duration::from_secs(5));
    // Drop owns kill/wait, including on the failing baseline.
    drop(binding);
    assert!(
        write.is_ok(),
        "retiring the launcher closed the provider input: {write:?}"
    );
    assert!(
        matches!(response, Ok(ClaudeRuntimeMessage::Stdout(value)) if value == json!({"ping": "after-thread-exit"})),
        "provider must still answer after its launching thread exits"
    );
}
