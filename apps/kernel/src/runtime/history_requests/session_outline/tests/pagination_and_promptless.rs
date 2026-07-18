use super::*;

#[test]
fn agent_outline_makes_legacy_duplicate_turn_ids_unique() {
    let path = std::env::temp_dir().join(format!(
        "arroba-duplicate-turn-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("prompt-2".to_string()),
        prompt_id: Some("prompt-2".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let first_prompt = HistoryEvent::transcript(
        10,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "first prompt"),
        context.clone(),
    );
    let second_prompt = HistoryEvent::transcript(
        20,
        &SessionHistoryEntry::user_prompt("session-1", "attachment-2", "agent-1", "second prompt"),
        context,
    );
    store
        .append(&first_prompt)
        .expect("first prompt should append");
    store
        .append(&second_prompt)
        .expect("second prompt should append");

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 2, None).expect("outline should load");

    assert_eq!(outline.turns.len(), 2);
    assert_eq!(outline.turns[0].turn_id, "prompt-2");
    assert_eq!(outline.turns[0].prompt_id.as_deref(), Some("prompt-2"));
    assert_eq!(outline.turns[0].user_prompt.entry.text, "first prompt");
    assert_eq!(outline.turns[1].turn_id, "prompt-2:seq-20");
    assert_eq!(outline.turns[1].prompt_id.as_deref(), Some("prompt-2"));
    assert_eq!(outline.turns[1].user_prompt.entry.text, "second prompt");

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_keeps_steering_prompts_inside_turns() {
    let path = std::env::temp_dir().join(format!(
        "arroba-steering-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let first_context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let second_context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-2".to_string()),
        prompt_id: Some("prompt-2".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    store
        .append(&HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                "attachment-1",
                "agent-1",
                "first prompt",
            ),
            first_context.clone(),
        ))
        .expect("first prompt should append");
    store
        .append(&HistoryEvent::operational(
            20,
            HistoryEventKind::UserPrompt,
            Some(crate::history::HistoryEventRole::User),
            Some("steer this turn".to_string()),
            std::collections::BTreeMap::from([
                (
                    "merge_key".to_string(),
                    serde_json::Value::String(crate::history::steering_prompt_merge_key(
                        "queued-1",
                    )),
                ),
                (
                    "source_attachment_id".to_string(),
                    serde_json::Value::String("attachment-1".to_string()),
                ),
            ]),
            first_context,
        ))
        .expect("steering prompt should append");
    store
        .append(&HistoryEvent::transcript(
            30,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                "attachment-1",
                "agent-1",
                "second prompt",
            ),
            second_context,
        ))
        .expect("second prompt should append");

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 2, None).expect("outline should load");

    assert_eq!(outline.turns.len(), 2);
    assert_eq!(outline.turns[0].user_prompt.entry.text, "first prompt");
    assert_eq!(outline.turns[0].entries.len(), 1);
    assert_eq!(outline.turns[0].entries[0].entry.text, "steer this turn");
    assert_eq!(
        outline.turns[0].entries[0].entry.merge_key.as_deref(),
        Some("steering-prompt:queued-1")
    );
    assert_eq!(outline.turns[1].user_prompt.entry.text, "second prompt");
    assert_eq!(outline.turns[1].entries.len(), 0);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_pages_older_turns_with_cursor() {
    let path = std::env::temp_dir().join(format!(
        "arroba-cursor-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    for index in 1..=5 {
        let sequence = index * 10;
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some(format!("turn-{index}")),
            prompt_id: Some(format!("prompt-{index}")),
            ..HistoryEventTurnContext::default()
        };
        let prompt = HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::user_prompt(
                "session-1",
                &format!("attachment-{index}"),
                "agent-1",
                &format!("prompt {index}"),
            ),
            context,
        );
        store.append(&prompt).expect("prompt should append");
    }

    let newest = load_agent_outline(&store, "session-1", "agent-1", 2, None)
        .expect("newest page should load");
    assert_eq!(newest.turns.len(), 2);
    assert_eq!(newest.turns[0].user_prompt.entry.text, "prompt 4");
    assert_eq!(newest.turns[1].user_prompt.entry.text, "prompt 5");
    assert_eq!(
        newest
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.before_sequence),
        Some(40)
    );

    let older = load_agent_outline(
        &store,
        "session-1",
        "agent-1",
        2,
        newest
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.before_sequence),
    )
    .expect("older page should load");
    assert_eq!(older.turns.len(), 2);
    assert_eq!(older.turns[0].user_prompt.entry.text, "prompt 2");
    assert_eq!(older.turns[1].user_prompt.entry.text, "prompt 3");
    assert_eq!(
        older
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.before_sequence),
        Some(20)
    );

    let oldest = load_agent_outline(
        &store,
        "session-1",
        "agent-1",
        2,
        older
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.before_sequence),
    )
    .expect("oldest page should load");
    assert_eq!(oldest.turns.len(), 1);
    assert_eq!(oldest.turns[0].user_prompt.entry.text, "prompt 1");
    assert_eq!(oldest.next_cursor, None);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_rehydrates_file_image_attachment_previews() {
    let image_path = std::env::temp_dir().join(format!(
        "arroba-outline-preview-{}-{}.png",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::write(&image_path, b"file-image").expect("fixture image should write");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let mut entry =
        SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "inspect image");
    entry.attachments = vec![SessionHistoryPromptAttachment {
        url: format!("file://{}", image_path.display()),
        mime: "image/png".to_string(),
        filename: Some("file-screenshot.png".to_string()),
        preview_url: None,
    }];
    let event = HistoryEvent::transcript(10, &entry, context);

    let page_entry = page_entry_from_event(event).expect("page entry should project");

    assert_eq!(
        page_entry
            .entry
            .attachments
            .first()
            .and_then(|attachment| attachment.preview_url.as_deref()),
        Some("data:image/png;base64,ZmlsZS1pbWFnZQ==")
    );

    let _ = std::fs::remove_file(image_path);
}

#[test]
fn agent_outline_leaves_promptless_local_provider_activity_empty() {
    let path = std::env::temp_dir().join(format!(
        "arroba-promptless-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let mut tool_entry = SessionHistoryEntry::provider_output(
        "session-1",
        "run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderTool,
        Some("tool-1".to_string()),
        r#"{"tool":"bash","status":"completed","input":{"command":"cargo test"}}"#,
    );
    tool_entry.timestamp_ms = 1_234;
    let tool = HistoryEvent::transcript(1, &tool_entry, context);
    store
        .append(&tool)
        .expect("promptless provider activity should append");

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 1, None).expect("outline should load");

    assert!(outline.turns.is_empty());
    assert_eq!(outline.next_cursor, None);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_omits_persisted_external_recovery_envelopes() {
    let path = std::env::temp_dir().join(format!(
        "arroba-recovery-envelope-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("recovery-turn".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let recovery_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "[Arroba recovery operation arroba-recovery:prompt-1:1] Continue the active task.",
        "codex",
        "thread-1",
        Some("observed-recovery".to_string()),
        Some(1),
    );
    let recovery_tool = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderTool,
        r#"{"tool":"bash","input":{"command":"internal"}}"#,
        "codex",
        "thread-1",
        Some("observed-tool".to_string()),
        Some(2),
    );
    store
        .append(&HistoryEvent::transcript(
            1,
            &recovery_prompt,
            context.clone(),
        ))
        .expect("recovery prompt should persist");
    store
        .append(&HistoryEvent::transcript(2, &recovery_tool, context))
        .expect("recovery tool should persist");

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 4, None).expect("outline should load");

    assert!(outline.turns.is_empty());
    assert_eq!(outline.next_cursor, None);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn session_history_outline_recovers_multiple_agents_and_image_attachment_after_store_restart() {
    let path = std::env::temp_dir().join(format!(
        "arroba-history-restart-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    {
        let store = OperationalHistoryStore::open(path.clone())
            .expect("operational history store should open before restart");
        let promptless_context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-primary".to_string()),
            turn_id: Some("turn-primary".to_string()),
            provider_run_id: Some("run-primary".to_string()),
            provider: Some("codex".to_string()),
            model: Some("gpt-5".to_string()),
            ..HistoryEventTurnContext::default()
        };
        store
            .append(&HistoryEvent::transcript(
                10,
                &SessionHistoryEntry::provider_output(
                    "session-1",
                    "run-primary",
                    Some("agent-primary"),
                    TerminalOutputKind::ProviderOutput,
                    Some("assistant-primary".to_string()),
                    "recovered promptless output",
                ),
                promptless_context,
            ))
            .expect("promptless output should persist");

        let image_context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-image".to_string()),
            turn_id: Some("turn-image".to_string()),
            prompt_id: Some("prompt-image".to_string()),
            provider_run_id: Some("run-image".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let mut image_prompt = SessionHistoryEntry::user_prompt(
            "session-1",
            "attachment-image",
            "agent-image",
            "inspect this image",
        );
        image_prompt.attachments = vec![SessionHistoryPromptAttachment {
            url: "data:image/png;base64,aW1hZ2U=".to_string(),
            mime: "image/png".to_string(),
            filename: Some("screen.png".to_string()),
            preview_url: Some("data:image/png;base64,aW1hZ2U=".to_string()),
        }];
        store
            .append(&HistoryEvent::transcript(
                20,
                &image_prompt,
                image_context.clone(),
            ))
            .expect("image prompt should persist");
        store
            .append(&HistoryEvent::transcript(
                21,
                &SessionHistoryEntry::provider_output(
                    "session-1",
                    "run-image",
                    Some("agent-image"),
                    TerminalOutputKind::ProviderOutput,
                    Some("assistant-image".to_string()),
                    "image response",
                ),
                image_context,
            ))
            .expect("image response should persist");
    }

    let restored = OperationalHistoryStore::open(path.clone())
        .expect("operational history store should reopen after restart");
    let promptless = load_agent_outline(&restored, "session-1", "agent-primary", 4, None)
        .expect("promptless provider activity should reload after restart");
    let image = load_agent_outline(&restored, "session-1", "agent-image", 4, None)
        .expect("image history should reload after restart");

    assert!(promptless.turns.is_empty());
    assert_eq!(image.turns.len(), 1);
    assert_eq!(image.turns[0].user_prompt.entry.text, "inspect this image");
    assert_eq!(
        image.turns[0].user_prompt.entry.attachments[0]
            .filename
            .as_deref(),
        Some("screen.png")
    );
    assert_eq!(
        image.turns[0]
            .summary
            .as_ref()
            .map(|entry| entry.entry.text.as_str()),
        Some("image response")
    );

    drop(restored);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_leaves_promptless_imported_provider_activity_empty() {
    let path = std::env::temp_dir().join(format!(
        "arroba-promptless-outline-pages-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    for index in 1..=3 {
        let context = HistoryEventTurnContext {
            session_id: Some("session-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            turn_id: Some(format!("turn-{index}")),
            provider_run_id: Some(format!("run-{index}")),
            provider: Some("codex".to_string()),
            model: Some("gpt-5".to_string()),
            ..HistoryEventTurnContext::default()
        };
        let output = HistoryEvent::transcript(
            index * 10,
            &SessionHistoryEntry::provider_output(
                "session-1",
                &format!("run-{index}"),
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                Some(format!("merge-{index}")),
                format!("promptless output {index}"),
            ),
            context,
        );
        store
            .append(&output)
            .expect("promptless provider activity should append");
    }

    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
    let latest = load_scoped_agent_outline(&store, "session-1", "agent-1", 2, None, Some(&import))
        .expect("latest outline should load");

    assert!(latest.turns.is_empty());
    assert_eq!(latest.next_cursor, None);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_leaves_promptless_observed_activity_empty() {
    let path = std::env::temp_dir().join(format!(
        "arroba-promptless-external-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        provider: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        ..HistoryEventTurnContext::default()
    };
    let tool_entry = SessionHistoryEntry::external_provider_observed(
        "session-1",
        Some("run-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderTool,
        r#"{"tool":"bash","status":"completed","input":{"command":"cargo test"}}"#,
        "codex",
        "thread-1",
        Some("tool-1".to_string()),
        Some(42),
    );
    let tool = HistoryEvent::transcript(1, &tool_entry, context);
    store
        .append(&tool)
        .expect("promptless external activity should append");

    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");
    let outline = load_scoped_agent_outline(&store, "session-1", "agent-1", 1, None, Some(&import))
        .expect("outline should load");

    assert!(outline.turns.is_empty());
    assert_eq!(outline.next_cursor, None);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_caps_large_provider_output_inline_payloads() {
    let path = std::env::temp_dir().join(format!(
        "arroba-large-output-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    store
        .append(&HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "prompt"),
            context.clone(),
        ))
        .expect("prompt should append");
    let large_output = "x".repeat(MAX_OUTLINE_INLINE_CHARS + 1024);
    store
        .append(&HistoryEvent::transcript(
            20,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                Some("merge-1".to_string()),
                large_output.clone(),
            ),
            context,
        ))
        .expect("large provider output should append");

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 1, None).expect("outline should load");

    assert_eq!(outline.turns.len(), 1);
    let turn = &outline.turns[0];
    let summary = turn.summary.as_ref().expect("summary should be present");
    assert_eq!(summary.total_chars, large_output.chars().count());
    assert_eq!(summary.fragment_end, MAX_OUTLINE_INLINE_CHARS);
    assert_eq!(summary.entry.text.chars().count(), MAX_OUTLINE_INLINE_CHARS);
    assert!(turn.entries.is_empty());
    assert_eq!(turn.blobs.len(), 1);
    assert_eq!(turn.blobs[0].kind, SessionHistoryEntryKind::ProviderOutput);
    assert_eq!(turn.blobs[0].total_chars, large_output.chars().count());

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_groups_large_blob_metadata_sets() {
    let path = std::env::temp_dir().join(format!(
        "arroba-large-blob-metadata-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    store
        .append(&HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "prompt"),
            context.clone(),
        ))
        .expect("prompt should append");
    for index in 0..(MAX_OUTLINE_EVENTS_PER_BLOB + 17) {
        let event = HistoryEvent::transcript(
            20 + index as u64,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderTool,
                Some(format!("tool-{index}")),
                format!(r#"{{"tool":"bash","index":{index}}}"#),
            ),
            context.clone(),
        );
        store.append(&event).expect("tool output should append");
    }

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 1, None).expect("outline should load");

    assert_eq!(outline.turns.len(), 1);
    let turn = &outline.turns[0];
    assert_eq!(turn.blobs.len(), 2);
    assert_eq!(turn.blobs[0].entry_count, MAX_OUTLINE_EVENTS_PER_BLOB);
    assert_eq!(turn.blobs[1].entry_count, 17);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
