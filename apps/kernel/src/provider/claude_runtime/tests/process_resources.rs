use super::*;
use crate::provider::claude_runtime::{drain_claude_events, process::write_json_line};
use std::time::{Duration, Instant};

// MP-08 / MP-09 / MP-10 / MP-11: exercise the stream runtime's real launch and
// drain path without touching a provider login. Real Claude acceptance is separate.
#[cfg(unix)]
#[test]
fn claude_process_resources_end_only_the_overweight_stream_and_name_the_limit() {
    let _environment = crate::env_lock::lock();
    let previous = std::env::var_os("CHARIOX_PROCESS_MEMORY_LIMIT_MIB");
    std::env::set_var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB", "32");
    let run = |id: &str, allocation: &str| {
        RuntimeProviderRun::new(
            id,
            &LaunchProviderRequest::new("resource-room", "claude", "claude", "default", "haiku"),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "claude:stream-json".into(),
                pty_target: None,
                pty_program: Some("python3".into()),
                pty_args: vec![
                    "-c".into(),
                    format!("import sys,time; sys.stdin.readline(); {allocation}; time.sleep(30)"),
                ],
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    };
    let heavy_run = run("claude-resource-heavy", "x=bytearray(64*1024*1024)");
    let sibling_run = run("claude-resource-sibling", "x=bytearray(1024)");
    let mut heavy =
        initialize_claude_runtime(&heavy_run).expect("Claude stream launch should succeed");
    let mut sibling =
        initialize_claude_runtime(&sibling_run).expect("unrelated stream should launch");
    heavy.state.active_turn_id = Some("resource-turn".into());
    write_json_line(
        &mut heavy.state.stdin,
        &json!({"type":"user","message":{"role":"user","content":"run bounded allocation"}}),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut result = ProviderPromptSignalBatch::default();
    while Instant::now() < deadline {
        result = drain_claude_events(&heavy_run, &mut heavy.state).unwrap();
        if result.terminal_failure.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let sibling_alive = sibling.state.child.try_wait().unwrap().is_none();
    drop(heavy);
    drop(sibling);
    match previous {
        Some(value) => std::env::set_var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB", value),
        None => std::env::remove_var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB"),
    }
    assert!(
        result.prompt_completed,
        "a memory-limited Claude turn must settle"
    );
    let reason = result.terminal_failure.unwrap_or_default();
    assert!(
        reason.contains("memory limit") && reason.contains("33554432"),
        "the Claude stream must name its 32 MiB limit: {reason}"
    );
    assert!(
        sibling_alive,
        "the unrelated Claude stream must remain alive"
    );
}
