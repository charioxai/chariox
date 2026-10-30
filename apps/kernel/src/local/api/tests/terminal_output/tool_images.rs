use super::*;

// MP-08 / MP-10: a tool result image over the 16 KB record cap reaches the live
// client and persisted history whole, with no truncation marker inside it.
#[test]
fn tool_result_images_survive_fanout_and_history_whole() {
    let worktree = crate::test_support::TestWorktree::new("terminal-tool-result-images");
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let run_id =
        harness.with_app_mut(|app| launch_slow_structured_run(app, session.id(), agent.id()));
    let image = format!("data:image/png;base64,{}", "iVBORw0KGgo=".repeat(8 * 1024));
    assert!(image.len() > 64 * 1024);
    let too_big = format!("data:image/png;base64,{}", "A".repeat(3 * 1024 * 1024));
    let payload = serde_json::json!({
        "id": "shot", "tool": "Read", "status": "completed", "output": "caption",
        "images": [image, image, image, too_big],
    });

    let record = harness.with_app_mut(|app| {
        app.fan_out_output_for_agent(
            session.id(),
            &run_id,
            Some(agent.id()),
            TerminalOutputKind::ProviderTool,
            Some("shot".to_string()),
            Vec::new(),
            payload.to_string().as_bytes(),
        )
    });
    let history = harness
        .with_app(|app| app.load_session_history_entries(&session, Some(agent.id())))
        .expect("history should load");
    let persisted = history
        .iter()
        .find(|entry| entry.merge_key.as_deref() == Some("shot"))
        .expect("the tool result is in history");

    for text in [
        String::from_utf8(record.bytes).expect("live record is utf8"),
        persisted.text.clone(),
    ] {
        let value: serde_json::Value = serde_json::from_str(&text).expect("tool json");
        assert_eq!(value["images"], serde_json::json!([image, image, image]));
        assert_eq!(value["images_omitted"], 1);
        assert_eq!(value["output"], "caption");
        assert!(!text.contains("chariox: output truncated"));
    }
}
