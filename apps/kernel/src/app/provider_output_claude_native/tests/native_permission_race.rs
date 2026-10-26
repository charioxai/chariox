//! MP-08 / MP-10 / MP-11: real permission race's reversed arrival order.
use super::*;

#[test]
fn official_permission_hook_owns_the_decision_when_rendered_frame_arrives_first() {
    let root = crate::test_support::TestWorktree::new("native-permission-hook-first");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let context = root.path().join("hidden-context.txt");
    let events = root.path().join("events.jsonl");
    fs::write(&context, "").unwrap();
    fs::write(&events, "").unwrap();
    let run = startup_readiness_run(
        "session-1",
        "agent-1",
        "run-permission-race",
        &context,
        &events,
        "cat".into(),
    );
    let mut value = serde_json::to_value(run).unwrap();
    value["permission_level"] = serde_json::json!("required");
    value["pty_env"]["CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES"] = serde_json::json!(root
        .path()
        .join("permission-responses")
        .display()
        .to_string());
    let run: RuntimeProviderRun = serde_json::from_value(value).unwrap();
    let bridge = RecordingPermissionBridge::default();
    let bridge_ref: std::sync::Arc<dyn ProviderNativeInteractionBridge> =
        std::sync::Arc::new(bridge.clone());
    let mut output = ProviderOutputClaudeNativeBridge::new(&mut app);
    output
        .process_terminal_output(
            "session-1",
            run.id(),
            &run,
            Some(bridge_ref.clone()),
            "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        bridge.interaction_ids.lock().unwrap().is_empty(),
        "rendered frame claimed the official hook's permission subject"
    );
    output.resolve_permission_event("session-1",run.id(),"agent-1",context.to_str().unwrap(),&run,Some(bridge_ref),
        &serde_json::json!({"hook_event_name":"PermissionRequest","hook_context_request_id":"current-permission","tool_name":"Bash","tool_input":{"command":"echo test"}})).unwrap();
    let response = root
        .path()
        .join("permission-responses/current-permission.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !response.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "official hook did not receive a decision"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        *bridge.interaction_ids.lock().unwrap(),
        vec!["claude-native-permission-run-permission-race-current-permission"]
    );
    let decision: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(response).unwrap()).unwrap();
    assert_eq!(decision["behavior"], "allow");
}
