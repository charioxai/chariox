use super::*;

#[test]
fn reads_opencode_observed_json_turns() {
    let temp = temp_dir("opencode-observed-turns");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("session-1.json"),
            r#"{"id":"open-1","messages":[{"id":"u1","role":"user","content":"Draft the OpenCode import drill.","createdAt":"2026-03-01T00:00:01.000Z"},{"id":"a1","role":"assistant","content":"Capture the waiting-room evidence.","createdAt":"2026-03-01T00:00:02.000Z"}]}"#,
        )
        .unwrap();

    let turns = read_opencode_observed_turns(root, "open-1");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u1"));
    assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Assistant);
    assert_eq!(turns[1].text, "Capture the waiting-room evidence.");
}

#[test]
fn reads_changed_opencode_jsonl_turns_incrementally_after_warmup() {
    let temp = temp_dir("opencode-observed-jsonl-incremental");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    let transcript = session_dir.join("session-1.jsonl");
    fs::write(
            &transcript,
            concat!(
                "{\"sessionID\":\"open-jsonl-1\",\"id\":\"u1\",\"role\":\"user\",\"content\":\"Draft the OpenCode JSONL import drill.\",\"createdAt\":\"2026-03-01T00:00:01.000Z\"}\n",
                "{\"sessionID\":\"open-jsonl-1\",\"id\":\"a1\",\"role\":\"assistant\",\"content\":\"Capture the first waiting-room evidence.\",\"createdAt\":\"2026-03-01T00:00:02.000Z\"}\n",
            ),
        )
        .unwrap();

    reset_jsonl_read_counts();
    let first = read_opencode_observed_turns(root, "open-jsonl-1");
    assert_eq!(jsonl_prefix_read_count(), 1);
    assert_eq!(jsonl_recent_read_count(), 1);
    assert_eq!(jsonl_incremental_read_count(), 0);
    assert_eq!(first.len(), 2);

    let mut file = OpenOptions::new().append(true).open(&transcript).unwrap();
    writeln!(
            file,
            "{{\"sessionID\":\"open-jsonl-1\",\"id\":\"a2\",\"role\":\"assistant\",\"content\":\"OPENCODE_INCREMENTAL_REPLY\",\"createdAt\":\"2026-03-01T00:00:03.000Z\"}}"
        )
        .unwrap();

    reset_jsonl_read_counts();
    let appended = read_opencode_observed_turns(root, "open-jsonl-1");
    assert_eq!(jsonl_prefix_read_count(), 0);
    assert_eq!(jsonl_recent_read_count(), 0);
    assert_eq!(jsonl_incremental_read_count(), 1);
    assert!(appended
        .iter()
        .any(|turn| turn.text == "OPENCODE_INCREMENTAL_REPLY"));
}

#[test]
fn reads_opencode_observed_message_parts_and_completion_metadata() {
    let temp = temp_dir("opencode-observed-parts");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("session-1.json"),
            r#"{
              "id": "open-1",
              "messages": [
                {
                  "info": { "id": "msg-user", "sessionID": "open-1", "role": "user" },
                  "parts": [{ "id": "part-user", "type": "text", "text": "Create, inspect, and delete a file." }]
                },
                {
                  "info": {
                    "id": "msg-assistant",
                    "sessionID": "open-1",
                    "role": "assistant",
                    "providerID": "moonshot",
                    "modelID": "kimi-k2-6",
                    "finish": "stop",
                    "tokens": { "input": 10, "output": 5, "reasoning": 2 },
                    "time": { "completed": 1782113000000 }
                  },
                  "parts": [
                    { "id": "part-reasoning", "type": "reasoning", "text": "Planning file changes." },
                    {
                      "id": "part-tool",
                      "type": "tool",
                      "tool": "bash",
                      "state": {
                        "status": "completed",
                        "input": { "command": "printf alpha > drill.txt" },
                        "output": "created"
                      }
                    },
                    { "id": "part-answer", "type": "text", "text": "Done." }
                  ]
                }
              ]
            }"#,
        )
        .unwrap();

    let turns = read_opencode_observed_turns(root, "open-1");
    assert_eq!(
        turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
        vec![
            ObservedExternalProviderTurnRole::User,
            ObservedExternalProviderTurnRole::Reasoning,
            ObservedExternalProviderTurnRole::Tool,
            ObservedExternalProviderTurnRole::Assistant,
            ObservedExternalProviderTurnRole::Status,
        ]
    );
    assert_eq!(turns[0].text, "Create, inspect, and delete a file.");
    assert_eq!(turns[1].text, "Planning file changes.");
    assert!(turns[2].text.contains("bash"));
    assert!(turns[2].text.contains("printf alpha > drill.txt"));
    assert!(turns[2].text.contains("created"));
    assert_eq!(turns[3].text, "Done.");
    assert!(turns[4].text.contains("opencode message completed"));
    assert!(turns[4].text.contains("kimi-k2-6"));
}

#[test]
fn reads_opencode_observed_sqlite_turns() {
    let temp = temp_dir("opencode-sqlite-observed-turns");
    let root = temp.path();
    let db_path = root.join("opencode.db");
    seed_opencode_sqlite(&db_path);

    let turns = read_opencode_observed_turns(root, "ses_sqlite_1");
    assert_eq!(turns.len(), 5);
    assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(turns[0].text, "Investigate SQLite-backed OpenCode imports.");
    assert_eq!(turns[0].provider_turn_id.as_deref(), Some("prt_user_text"));
    assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Reasoning);
    assert_eq!(turns[1].text, "Internal reasoning");
    assert_eq!(turns[1].provider_turn_id.as_deref(), Some("prt_reasoning"));
    assert_eq!(turns[2].role, ObservedExternalProviderTurnRole::Tool);
    assert!(turns[2].text.contains("TOOL_STEP_01"));
    assert!(turns[2].text.contains("created"));
    assert_eq!(turns[2].provider_turn_id.as_deref(), Some("prt_tool"));
    assert_eq!(turns[3].role, ObservedExternalProviderTurnRole::Assistant);
    assert_eq!(turns[3].text, "Use the session, message, and part tables.");
    assert_eq!(
        turns[3].provider_turn_id.as_deref(),
        Some("prt_assistant_text")
    );
    assert_eq!(turns[4].role, ObservedExternalProviderTurnRole::Status);
    assert_eq!(
        turns[4].provider_turn_id.as_deref(),
        Some("message-status-msg_assistant")
    );
    assert!(turns[4].text.contains("opencode message completed"));
    assert!(turns[4].text.contains("kimi-k2.6"));
}
