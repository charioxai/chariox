use super::*;

#[test]
fn reads_codex_observed_user_and_assistant_turns() {
    let temp = temp_dir("codex-observed-turns");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Plan the importer tests.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"a1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Use fixture transcripts.\"}]}}\n",
            ),
        )
        .unwrap();

    let turns = read_codex_observed_turns(root, "thread-1");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(turns[0].text, "Plan the importer tests.");
    assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u1"));
    assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Assistant);
    assert_eq!(turns[1].text, "Use fixture transcripts.");
}

#[test]
fn reads_codex_event_user_message_as_prompt_anchor() {
    let temp = temp_dir("codex-observed-event-user-message");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Plan the importer tests.\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"Use fixture transcripts.\"}}\n",
            ),
        )
        .unwrap();

    let turns = read_codex_observed_turns(root, "thread-1");
    assert_eq!(
        turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
        vec![
            ObservedExternalProviderTurnRole::User,
            ObservedExternalProviderTurnRole::Assistant,
        ]
    );
    assert_eq!(turns[0].text, "Plan the importer tests.");
    assert!(turns[0]
        .provider_turn_id
        .as_deref()
        .is_some_and(|id| id.starts_with("user-message-")));
}

#[test]
fn reads_codex_observed_reasoning_tools_and_status_metadata() {
    let temp = temp_dir("codex-observed-metadata");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-1\",\"model_context_window\":258400}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Create, inspect, and delete a file.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:03.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"r1\",\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"Planning file changes.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:04.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"printf alpha > drill.txt\\\"}\",\"call_id\":\"call-create\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:05.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call-create\",\"output\":\"created\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:06.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"a1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:07.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":1,\"output_tokens\":2}}}}\n",
            ),
        )
        .unwrap();

    let turns = read_codex_observed_turns(root, "thread-1");
    assert_eq!(
        turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
        vec![
            ObservedExternalProviderTurnRole::Status,
            ObservedExternalProviderTurnRole::User,
            ObservedExternalProviderTurnRole::Reasoning,
            ObservedExternalProviderTurnRole::Tool,
            ObservedExternalProviderTurnRole::Tool,
            ObservedExternalProviderTurnRole::Assistant,
            ObservedExternalProviderTurnRole::Status,
        ]
    );
    assert!(turns[0].text.contains("codex task_started"));
    assert!(turns[2].text.contains("Planning file changes."));
    assert!(turns[3].text.contains("exec_command"));
    assert!(turns[3].text.contains("printf alpha > drill.txt"));
    assert!(turns[4].text.contains("created"));
    assert!(turns[6].text.contains("total_token_usage"));
}

#[test]
fn reads_codex_observed_metadata_bounds_large_tool_payloads() {
    let temp = temp_dir("codex-observed-bounded-metadata");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    let large_output = "x".repeat(MAX_OBSERVED_METADATA_TEXT_CHARS * 2);
    let line = serde_json::json!({
        "timestamp": "2026-01-01T00:00:05.000Z",
        "type": "response_item",
        "payload": {
            "type": "function_call_output",
            "call_id": "call-large",
            "output": large_output,
        }
    });
    fs::write(
        session_dir.join("rollout.jsonl"),
        format!(
            "{}\n{}\n",
            serde_json::json!({
                "timestamp": "2026-01-01T00:00:00.000Z",
                "type": "session_meta",
                "payload": {"id": "thread-large", "cwd": "/repo"},
            }),
            line
        ),
    )
    .unwrap();

    let turns = read_codex_observed_turns(root, "thread-large");

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::Tool);
    assert!(turns[0].text.len() <= MAX_OBSERVED_METADATA_TEXT_CHARS + 3);
    assert!(turns[0].text.contains("arroba truncated"));
    assert!(turns[0].text.contains("call-large"));
}

#[test]
fn reads_codex_observed_turns_from_recent_jsonl_tail() {
    let temp = temp_dir("codex-observed-tail");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    let mut lines = vec![
            "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-tail\",\"cwd\":\"/repo\"}}".to_string(),
        ];
    for index in 0..320 {
        lines.push(format!(
                "{{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{{\"id\":\"noise-{index}\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"noise {index}\"}}]}}}}"
            ));
    }
    lines.push("{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u-tail\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Latest external prompt.\"}]}}".to_string());
    fs::write(
        session_dir.join("rollout.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();

    let turns = read_codex_observed_turns(root, "thread-tail");
    assert_eq!(turns.len(), MAX_OBSERVED_TURNS);
    assert_eq!(
        turns.last().map(|turn| turn.text.as_str()),
        Some("Latest external prompt.")
    );
    assert_eq!(
        turns
            .last()
            .and_then(|turn| turn.provider_turn_id.as_deref()),
        Some("u-tail")
    );
}

#[test]
fn reads_codex_observed_turns_from_indexed_path_before_candidate_scan() {
    let temp = temp_dir("codex-observed-index");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("indexed-target.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-indexed\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"indexed-user\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Indexed external prompt.\"}]}}\n",
            ),
        )
        .unwrap();

    let sessions = discover_codex_external_sessions(root);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].external_session_id, "codex:thread-indexed");

    for index in 0..=MAX_PROVIDER_FILES {
        fs::write(
                session_dir.join(format!("newer-decoy-{index}.jsonl")),
                format!(
                    "{{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"decoy-{index}\"}}}}\n"
                ),
            )
            .unwrap();
    }

    let turns = read_codex_observed_turns(root, "thread-indexed");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].text, "Indexed external prompt.");
    assert_eq!(turns[0].provider_turn_id.as_deref(), Some("indexed-user"));
}

#[test]
fn reads_codex_observed_turns_from_unchanged_index_without_jsonl_reads() {
    let temp = temp_dir("codex-observed-unchanged-index");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    let transcript = session_dir.join("indexed-unchanged.jsonl");
    fs::write(
            &transcript,
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-unchanged-index\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"indexed-user\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Indexed prompt before cache.\"}]}}\n",
            ),
        )
        .unwrap();

    let sessions = discover_codex_external_sessions(root);
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].external_session_id,
        "codex:thread-unchanged-index"
    );

    reset_jsonl_read_counts();
    let first = read_codex_observed_turns(root, "thread-unchanged-index");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].text, "Indexed prompt before cache.");
    assert_eq!(jsonl_prefix_read_count(), 0);
    assert_eq!(jsonl_recent_read_count(), 1);
    assert_eq!(jsonl_incremental_read_count(), 0);

    reset_jsonl_read_counts();
    let unchanged = read_codex_observed_turns(root, "thread-unchanged-index");
    assert_eq!(unchanged, first);
    assert_eq!(jsonl_prefix_read_count(), 0);
    assert_eq!(jsonl_recent_read_count(), 0);
    assert_eq!(jsonl_incremental_read_count(), 0);

    let mut file = OpenOptions::new().append(true).open(&transcript).unwrap();
    writeln!(
            file,
            "{{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{{\"id\":\"indexed-assistant\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"Indexed assistant after append.\"}}]}}}}"
        )
        .unwrap();

    reset_jsonl_read_counts();
    let appended = read_codex_observed_turns(root, "thread-unchanged-index");
    assert_eq!(jsonl_prefix_read_count(), 0);
    assert_eq!(jsonl_recent_read_count(), 0);
    assert_eq!(jsonl_incremental_read_count(), 1);
    assert!(appended
        .iter()
        .any(|turn| turn.text == "Indexed assistant after append."));
}

#[test]
fn reads_codex_observed_turns_does_not_advance_offset_past_partial_trailing_jsonl() {
    let temp = temp_dir("codex-observed-partial-tail");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    let transcript = session_dir.join("partial-tail.jsonl");
    let prefix = concat!(
        "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-partial-tail\",\"cwd\":\"/repo\"}}\n",
        "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"partial-user\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Prompt before partial tail.\"}]}}\n",
    );
    let assistant = concat!(
        "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"partial-assistant\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Assistant after partial tail.\"}]}}\n",
    );
    fs::write(&transcript, prefix).unwrap();

    let first = read_codex_observed_turns(root, "thread-partial-tail");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].provider_turn_id.as_deref(), Some("partial-user"));

    fs::write(
        &transcript,
        format!("{prefix}{}", &assistant[..assistant.len() / 2]),
    )
    .unwrap();
    let partial = read_codex_observed_turns(root, "thread-partial-tail");
    assert_eq!(
        partial, first,
        "partial trailing JSONL must not advance the observed offset"
    );

    fs::write(&transcript, format!("{prefix}{assistant}")).unwrap();
    let completed = read_codex_observed_turns(root, "thread-partial-tail");
    assert!(completed.iter().any(|turn| turn.provider_turn_id.as_deref()
        == Some("partial-assistant")
        && turn.text == "Assistant after partial tail."));
}

#[test]
fn reads_codex_observed_turns_preserves_latest_user_before_recent_tail() {
    let temp = temp_dir("codex-observed-tail-user");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    let mut lines = vec![
            "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-tail-user\",\"cwd\":\"/repo\"}}".to_string(),
            "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u-active\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Long running external prompt.\"}]}}".to_string(),
        ];
    for index in 0..MAX_OBSERVED_TURNS + 25 {
        lines.push(format!(
                "{{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"total_tokens\":{index}}}}}}}}}"
            ));
    }
    fs::write(
        session_dir.join("rollout.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();

    let turns = read_codex_observed_turns(root, "thread-tail-user");
    assert_eq!(turns.len(), MAX_OBSERVED_TURNS + 1);
    assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(turns[0].text, "Long running external prompt.");
    assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u-active"));
    assert!(turns[1..]
        .iter()
        .all(|turn| turn.role == ObservedExternalProviderTurnRole::Status));
}

#[test]
fn reads_codex_observed_turns_preserves_latest_user_before_recent_jsonl_window() {
    let temp = temp_dir("codex-observed-window-user");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    let mut lines = vec![
            "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-window-user\",\"cwd\":\"/repo\"}}".to_string(),
            "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u-window\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Very long external prompt turn.\"}]}}".to_string(),
        ];
    for index in 0..MAX_JSONL_LINES + 25 {
        lines.push(format!(
                "{{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"total_tokens\":{index}}}}}}}}}"
            ));
    }
    fs::write(
        session_dir.join("rollout.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();

    let turns = read_codex_observed_turns(root, "thread-window-user");
    assert_eq!(turns.len(), MAX_OBSERVED_TURNS + 1);
    assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
    assert_eq!(turns[0].text, "Very long external prompt turn.");
    assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u-window"));
    assert!(turns[1..]
        .iter()
        .all(|turn| turn.role == ObservedExternalProviderTurnRole::Status));
}

#[test]
fn reads_codex_observed_turns_deduplicates_mirrored_visible_events() {
    let temp = temp_dir("codex-observed-mirror-dedupe");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Run a drill.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.001Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Run a drill.\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"The drill passed.\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.001Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"msg_rich\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"The drill passed.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:03.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}}\n",
            ),
        )
        .unwrap();

    let turns = read_codex_observed_turns(root, "thread-1");
    assert_eq!(
        turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
        vec![
            ObservedExternalProviderTurnRole::User,
            ObservedExternalProviderTurnRole::Assistant,
            ObservedExternalProviderTurnRole::Status,
        ]
    );
    assert_eq!(turns[1].text, "The drill passed.");
    assert_eq!(turns[1].provider_turn_id.as_deref(), Some("msg_rich"));
    assert!(turns[2].text.contains("total_token_usage"));
}
