use super::*;
use crate::provider::claude_runtime::{drain_claude_events, process::ClaudeRuntimeMessage};

fn drain(messages: Vec<serde_json::Value>) -> ProviderPromptSignalBatch {
    let (mut state, _) = parser_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.receiver = receiver;
    let run = RuntimeProviderRun::new(
        "run-1",
        &LaunchProviderRequest::new("session-1", "claude", "claude", "default", "sonnet"),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "test-claude".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test-claude-runtime".to_string()),
        },
    );
    for message in messages {
        sender.send(ClaudeRuntimeMessage::Stdout(message)).unwrap();
    }
    drain_claude_events(&run, &mut state).expect("provider stream should drain")
}

#[test]
fn claude_stream_projects_matching_tool_result_error_and_input() {
    let batch = drain(vec![
        json!({"type":"assistant","message":{"role":"assistant","content":[{
            "type":"tool_use","id":"toolu_fill","name":"mcp__chariox__slice_browser_fill",
            "input":{"text":"STALE ATTEMPT MUST NOT LAND","element_ref":"old"}
        }]}}),
        json!({"type":"user","message":{"role":"user","content":[{
            "type":"tool_result","tool_use_id":"toolu_fill","is_error":true,
            "content":[{"type":"text","text":"environment_stale_element_reference"}]
        }]}}),
    ]);
    assert_eq!(
        batch.chunks.len(),
        2,
        "both invocation and result must reach history"
    );
    let start: serde_json::Value = serde_json::from_slice(&batch.chunks[0].bytes).unwrap();
    let result: serde_json::Value = serde_json::from_slice(&batch.chunks[1].bytes).unwrap();
    assert_eq!(batch.chunks[1].kind, TerminalOutputKind::ProviderTool);
    assert_eq!(batch.chunks[0].merge_key.as_deref(), Some("toolu_fill"));
    assert_eq!(batch.chunks[1].merge_key, batch.chunks[0].merge_key);
    assert_eq!(start["status"], "running");
    assert_eq!(result["tool"], "mcp__chariox__slice_browser_fill");
    assert_eq!(result["input"]["text"], "STALE ATTEMPT MUST NOT LAND");
    assert_eq!(result["error"], "environment_stale_element_reference");
    assert_eq!(result["status"], "error");
    assert!(
        !batch.prompt_completed,
        "a failed tool is not a failed provider turn"
    );
    assert!(batch.terminal_failure.is_none());
}
