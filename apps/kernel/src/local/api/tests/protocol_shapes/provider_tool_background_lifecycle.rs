use super::*;

// MP-08 / MP-10: every ProviderTool payload carries `started_at_ms`; a command
// left running past its call adds `background: true` and `finished_at_ms`,
// `null` until the kernel learns the command ended.
#[test]
fn mp10_provider_tool_background_lifecycle_snapshot_and_hash() {
    let bytes = include_bytes!("provider-tool-background-lifecycle.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "0f95cd4df55e69d0d097abb1633a485d088e118df46792bd1233109d0d083257"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local_protocol"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["peer_protocol"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let foreground = &fixture["claude_foreground_completed"];
    assert!(foreground["started_at_ms"].is_u64());
    assert!(foreground.get("background").is_none());
    assert!(foreground.get("finished_at_ms").is_none());
    for (running, finished) in [
        ("claude_background_launched", "claude_background_finished"),
        ("codex_background_running", "codex_background_finished"),
    ] {
        let (running, finished) = (&fixture[running], &fixture[finished]);
        assert_eq!(running["id"], finished["id"]);
        assert_eq!(running["background"], true);
        assert!(running["finished_at_ms"].is_null());
        assert_eq!(running["started_at_ms"], finished["started_at_ms"]);
        assert!(finished["finished_at_ms"].as_u64() > finished["started_at_ms"].as_u64());
    }
}

#[test]
fn mp10_provider_tool_lifecycle_fields_survive_the_terminal_caps_whole() {
    let fixture: serde_json::Value =
        serde_json::from_slice(include_bytes!("provider-tool-background-lifecycle.json")).unwrap();
    for name in ["claude_background_launched", "codex_background_finished"] {
        let mut payload = fixture[name].clone();
        payload["output"] = serde_json::json!("x".repeat(64 * 1024));
        let bounded = crate::provider_output_policy::output_bounds::bounded_terminal_output_bytes(
            &crate::terminal::TerminalOutputKind::ProviderTool,
            payload.to_string().as_bytes(),
        );
        let bounded: serde_json::Value = serde_json::from_slice(&bounded).unwrap();
        for field in ["id", "started_at_ms", "background", "finished_at_ms"] {
            assert_eq!(bounded[field], payload[field], "{name}.{field}");
        }
    }
}
