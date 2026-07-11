use super::*;

#[test]
fn reads_claude_observed_user_and_assistant_turns() {
    let temp = temp_dir("claude-observed-turns");
    let root = temp.path();
    let session_dir = root.join("projects").join("-repo");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("session-1.jsonl"),
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":[{\"text\":\"Summarize external imports.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
                "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"role\":\"assistant\",\"content\":[{\"text\":\"External imports reuse provider sessions.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:02.000Z\"}\n",
            ),
        )
        .unwrap();

    let turns = read_claude_observed_turns(root, "session-1");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(turns[0].text, "Summarize external imports.");
    assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Assistant);
    assert_eq!(turns[1].provider_turn_id.as_deref(), Some("a1"));
}

#[test]
fn reads_claude_observed_turns_preserves_latest_user_before_recent_jsonl_window() {
    let temp = temp_dir("claude-observed-window-user");
    let root = temp.path();
    let session_dir = root.join("projects").join("-repo");
    fs::create_dir_all(&session_dir).unwrap();
    let mut lines = vec![
            "{\"type\":\"user\",\"uuid\":\"u-window\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Run a long Claude external drill.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}".to_string(),
        ];
    for index in 0..MAX_JSONL_LINES + 25 {
        lines.push(format!(
                "{{\"type\":\"mode\",\"mode\":\"default\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:02.{index:03}Z\"}}"
            ));
    }
    lines.push(
            "{\"type\":\"assistant\",\"uuid\":\"a-final\",\"message\":{\"role\":\"assistant\",\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"FINAL_EXTERNAL_PARITY_SUMMARY\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:03.000Z\"}".to_string(),
        );
    fs::write(
        session_dir.join("session-1.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();

    let turns = read_claude_observed_turns(root, "session-1");
    assert_eq!(turns.len(), MAX_OBSERVED_TURNS + 1);
    assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(turns[0].text, "Run a long Claude external drill.");
    assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u-window"));
    assert!(turns
        .iter()
        .any(|turn| turn.text == "FINAL_EXTERNAL_PARITY_SUMMARY"));
    assert!(turns
        .iter()
        .any(|turn| turn.text.starts_with("claude message completed")));
}

#[test]
fn reads_changed_claude_observed_turns_reuses_cached_user_anchor() {
    let temp = temp_dir("claude-observed-cached-user");
    let root = temp.path();
    let session_dir = root.join("projects").join("-repo");
    fs::create_dir_all(&session_dir).unwrap();
    let transcript = session_dir.join("session-cached-user.jsonl");
    let mut lines = vec![
            "{\"type\":\"user\",\"uuid\":\"u-window\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Run a long cached Claude external drill.\"}]},\"sessionId\":\"session-cached-user\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}".to_string(),
        ];
    for index in 0..MAX_JSONL_LINES + 25 {
        lines.push(format!(
                "{{\"type\":\"mode\",\"mode\":\"default\",\"sessionId\":\"session-cached-user\",\"timestamp\":\"2026-02-01T00:00:02.{index:03}Z\"}}"
            ));
    }
    fs::write(&transcript, format!("{}\n", lines.join("\n"))).unwrap();

    reset_jsonl_read_counts();
    let first = read_claude_observed_turns(root, "session-cached-user");
    assert_eq!(jsonl_prefix_read_count(), 2);
    assert_eq!(jsonl_recent_read_count(), 1);
    assert_eq!(jsonl_incremental_read_count(), 0);
    assert_eq!(first[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(first[0].text, "Run a long cached Claude external drill.");

    let mut file = OpenOptions::new().append(true).open(&transcript).unwrap();
    writeln!(
            file,
            "{{\"type\":\"assistant\",\"uuid\":\"a-after-cache\",\"message\":{{\"role\":\"assistant\",\"stop_reason\":\"end_turn\",\"content\":[{{\"type\":\"text\",\"text\":\"CACHE_REUSED_CLAUDE_REPLY\"}}]}},\"sessionId\":\"session-cached-user\",\"timestamp\":\"2026-02-01T00:00:03.000Z\"}}"
        )
        .unwrap();

    reset_jsonl_read_counts();
    let appended = read_claude_observed_turns(root, "session-cached-user");
    assert_eq!(jsonl_prefix_read_count(), 0);
    assert_eq!(jsonl_recent_read_count(), 0);
    assert_eq!(jsonl_incremental_read_count(), 1);
    assert_eq!(appended[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(appended[0].text, "Run a long cached Claude external drill.");
    assert!(appended
        .iter()
        .any(|turn| turn.text == "CACHE_REUSED_CLAUDE_REPLY"));
}

#[test]
fn reads_claude_end_turn_as_completion_status() {
    let temp = temp_dir("claude-observed-completion");
    let root = temp.path();
    let session_dir = root.join("projects").join("-repo");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("session-1.jsonl"),
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Run a drill.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
                "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"role\":\"assistant\",\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"FINAL_EXTERNAL_PARITY_SUMMARY\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:02.000Z\"}\n",
                "{\"type\":\"last-prompt\",\"sessionId\":\"session-1\",\"leafUuid\":\"a1\"}\n",
            ),
        )
        .unwrap();

    let turns = read_claude_observed_turns(root, "session-1");
    assert_eq!(
        turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
        vec![
            ObservedExternalProviderTurnRole::User,
            ObservedExternalProviderTurnRole::Assistant,
            ObservedExternalProviderTurnRole::Status,
            ObservedExternalProviderTurnRole::Status,
        ]
    );
    assert_eq!(turns[1].text, "FINAL_EXTERNAL_PARITY_SUMMARY");
    assert_eq!(turns[2].provider_turn_id.as_deref(), Some("a1:completed"));
    assert!(turns[2].text.starts_with("claude message completed"));
    assert!(turns[2].text.contains("stop_reason"));
    assert!(turns[2].text.contains("end_turn"));
    assert_eq!(
        turns[3].provider_turn_id.as_deref(),
        Some("last-prompt-leaf-a1")
    );
    assert!(turns[3].text.starts_with("claude last-prompt"));
}

#[test]
fn reads_claude_observed_reasoning_tools_and_status_metadata() {
    let temp = temp_dir("claude-observed-metadata");
    let root = temp.path();
    let session_dir = root.join("projects").join("-repo");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("session-1.jsonl"),
            concat!(
                "{\"type\":\"mode\",\"mode\":\"default\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:00.000Z\"}\n",
                "{\"type\":\"permission-mode\",\"permissionMode\":\"acceptEdits\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:00.500Z\"}\n",
                "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Create, inspect, and delete a file.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
                "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"id\":\"msg-1\",\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"Planning file changes.\"},{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",\"input\":{\"command\":\"printf alpha > drill.txt\"}},{\"type\":\"text\",\"text\":\"I will inspect the file next.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:02.000Z\"}\n",
                "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"created\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:03.000Z\"}\n",
                "{\"type\":\"last-prompt\",\"prompt\":\"Create, inspect, and delete a file.\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:04.000Z\"}\n",
                "{\"type\":\"file-history-snapshot\",\"snapshot\":{\"large\":true},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:05.000Z\"}\n",
            ),
        )
        .unwrap();

    let turns = read_claude_observed_turns(root, "session-1");
    assert_eq!(
        turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
        vec![
            ObservedExternalProviderTurnRole::Status,
            ObservedExternalProviderTurnRole::Status,
            ObservedExternalProviderTurnRole::User,
            ObservedExternalProviderTurnRole::Reasoning,
            ObservedExternalProviderTurnRole::Tool,
            ObservedExternalProviderTurnRole::Assistant,
            ObservedExternalProviderTurnRole::Tool,
            ObservedExternalProviderTurnRole::Status,
        ]
    );
    assert!(turns[0].text.contains("claude mode"));
    assert!(turns[1].text.contains("permissionMode"));
    assert_eq!(turns[2].text, "Create, inspect, and delete a file.");
    assert_eq!(turns[3].text, "Planning file changes.");
    assert!(turns[4].text.contains("Bash"));
    assert!(turns[4].text.contains("printf alpha > drill.txt"));
    assert_eq!(turns[5].text, "I will inspect the file next.");
    assert!(turns[6].text.contains("tool_result"));
    assert!(turns[6].text.contains("created"));
    assert!(turns[7].text.contains("claude last-prompt"));
    assert!(turns
        .iter()
        .all(|turn| !turn.text.contains("file-history-snapshot")));
}
