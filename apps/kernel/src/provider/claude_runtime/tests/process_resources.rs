use super::*;
use crate::provider::claude_runtime::{drain_claude_events, process::write_json_line};
use std::time::{Duration, Instant};

// MP-08 / MP-09 / MP-10 / MP-11: real launch, parser and resource monitor;
// the common transcript tracker must outlive adapter-local turn cleanup.
#[cfg(unix)]
#[test]
fn claude_process_resources_end_only_the_overweight_stream_and_name_the_limit() {
    process_end_settles_pending_stream(true);
}

#[cfg(unix)]
#[test]
fn claude_process_resources_outside_end_settles_pending_stream() {
    process_end_settles_pending_stream(false);
}

#[cfg(unix)]
fn process_end_settles_pending_stream(memory_limit: bool) {
    let _environment = crate::env_lock::lock();
    let previous = std::env::var_os("CHARIOX_PROCESS_MEMORY_LIMIT_MIB");
    std::env::set_var("CHARIOX_PROCESS_MEMORY_LIMIT_MIB", "32");
    let invocation = json!({"type":"assistant","message":{"role":"assistant","content":[{
        "type":"tool_use","id":"resource-tool","name":"Bash","input":{"command":"bounded allocation"}
    }]}}).to_string();
    let run = |id: &str, allocation: &str| {
        RuntimeProviderRun::new(
            id,
            &LaunchProviderRequest::new("resource-room", "claude", "claude", "default", "haiku"),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "claude:stream-json".into(),
                pty_target: None,
                pty_program: Some("python3".into()),
                pty_args: vec!["-c".into(), format!(
                    "import sys,time; sys.stdin.readline(); print({invocation:?},flush=True); time.sleep(0.2); {allocation}; time.sleep(30)"
                )],
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        )
    };
    let heavy_run = run(
        "claude-resource-heavy",
        if memory_limit {
            "x=bytearray(64*1024*1024)"
        } else {
            "x=bytearray(1024)"
        },
    );
    let sibling_run = run("claude-resource-sibling", "x=bytearray(1024)");
    let mut heavy =
        initialize_claude_runtime(&heavy_run).expect("Claude stream launch should succeed");
    let mut sibling =
        initialize_claude_runtime(&sibling_run).expect("unrelated stream should launch");
    let activity = crate::app::PromptActivityStore::default();
    activity.write().insert(
        heavy_run.id().into(),
        crate::app::ActivePromptState {
            last_output_at: None,
            saw_response_content: false,
            completion_recorded: false,
            settlement_requested: false,
            active_tools: Default::default(),
        },
    );
    heavy.state.active_turn_id = Some("resource-turn".into());
    write_json_line(
        &mut heavy.state.stdin,
        &json!({"type":"user","message":{"role":"user","content":"run bounded allocation"}}),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut result = ProviderPromptSignalBatch::default();
    let mut saw_tool = false;
    let mut stopped = false;
    while Instant::now() < deadline {
        result = drain_claude_events(&heavy_run, &mut heavy.state).unwrap();
        for chunk in &result.chunks {
            if chunk.kind == TerminalOutputKind::ProviderTool {
                saw_tool = true;
                activity
                    .write()
                    .get_mut(heavy_run.id())
                    .unwrap()
                    .observe_provider_tool(chunk.merge_key.as_deref(), &chunk.bytes);
            }
        }
        if result.terminal_failure.is_some() {
            break;
        }
        if !memory_limit && saw_tool && !stopped {
            crate::runtime::process_resources::stop_for_key(heavy_run.id(), None).unwrap();
            stopped = true;
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
    assert!(result.prompt_completed, "an ended Claude turn must settle");
    assert!(
        saw_tool,
        "the real stream must start a tool before termination"
    );
    let reason = result.terminal_failure.unwrap_or_default();
    if memory_limit {
        assert!(
            reason.contains("memory limit") && reason.contains("33554432"),
            "the stream must name its 32 MiB limit: {reason}"
        );
    } else {
        assert!(
            result.process_outside_ended,
            "the stream must retain outside-end provenance"
        );
    }
    let updates = activity.take_failed_provider_tools(heavy_run.id(), &reason);
    assert_eq!(
        updates.len(),
        1,
        "adapter cleanup must not erase the common pending tool"
    );
    assert_eq!(updates[0].merge_key.as_deref(), Some("resource-tool"));
    let payload: serde_json::Value = serde_json::from_slice(&updates[0].bytes).unwrap();
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["error"], reason);
    assert_eq!(payload["tool"], "Bash");
    assert_eq!(payload["input"]["command"], "bounded allocation");
    assert!(activity
        .take_failed_provider_tools(heavy_run.id(), &reason)
        .is_empty());
    assert!(
        sibling_alive,
        "the unrelated Claude stream must remain alive"
    );
}
