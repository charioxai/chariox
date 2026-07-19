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

#[test]
fn opencode_sqlite_observation_cache_is_scoped_to_the_attached_session() {
    let temp = temp_dir(&format!(
        "opencode-sqlite-session-cache-{}",
        std::process::id()
    ));
    let root = temp.path();
    let db_path = root.join("opencode.db");
    seed_opencode_sqlite(&db_path);
    let provider_session_id = "ses_sqlite_cache_target";
    let connection = Connection::open(&db_path).expect("sqlite fixture should reopen");
    connection
        .execute(
            "update part set session_id = ?1 where session_id = 'ses_sqlite_1'",
            [provider_session_id],
        )
        .unwrap();
    connection
        .execute(
            "update message set session_id = ?1 where session_id = 'ses_sqlite_1'",
            [provider_session_id],
        )
        .unwrap();
    connection
        .execute(
            "update session set id = ?1 where id = 'ses_sqlite_1'",
            [provider_session_id],
        )
        .unwrap();
    drop(connection);

    let first = read_opencode_observed_turns(root, provider_session_id);
    assert_eq!(first.len(), 5);
    assert!(!external_provider_session_transcript_needs_refresh(
        "opencode",
        provider_session_id,
    ));

    let connection = Connection::open(&db_path).expect("sqlite fixture should reopen");
    connection
        .execute_batch(
            r#"
            insert into session (
                id, project_id, slug, directory, title, version,
                time_created, time_updated
            ) values (
                'ses_sqlite_unrelated', 'project_2', 'unrelated',
                '/repo/unrelated', 'Unrelated session', '0.0.0',
                1782114000000, 1782114000000
            );
            insert into message (id, session_id, time_created, time_updated, data)
            values (
                'msg_unrelated', 'ses_sqlite_unrelated', 1782114000001, 1782114000001,
                '{"role":"user"}'
            );
            insert into part (id, message_id, session_id, time_created, time_updated, data)
            values (
                'prt_unrelated', 'msg_unrelated', 'ses_sqlite_unrelated',
                1782114000001, 1782114000001, '{"type":"text","text":"Unrelated"}'
            );
            "#,
        )
        .unwrap();
    drop(connection);

    assert!(provider_transcript_file_fingerprint(&db_path).is_some());
    assert!(!external_provider_session_transcript_needs_refresh(
        "opencode",
        provider_session_id,
    ));

    let connection = Connection::open(&db_path).expect("sqlite fixture should reopen");
    connection
        .execute(
            "update part set data = ?1, time_updated = 1782115000000 where id = 'prt_assistant_text'",
            [r#"{"type":"text","text":"Session-specific update."}"#],
        )
        .unwrap();
    connection
        .execute(
            "update session set time_updated = 1782115000000 where id = ?1",
            [provider_session_id],
        )
        .unwrap();
    drop(connection);

    assert!(external_provider_session_transcript_needs_refresh(
        "opencode",
        provider_session_id,
    ));
    let refreshed = read_opencode_observed_turns(root, provider_session_id);
    assert!(refreshed
        .iter()
        .any(|turn| turn.text == "Session-specific update."));
    assert!(!external_provider_session_transcript_needs_refresh(
        "opencode",
        provider_session_id,
    ));
}
