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
fn agent_outline_synthesizes_turn_for_promptless_provider_activity() {
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
    let tool = HistoryEvent::transcript(
        1,
        &SessionHistoryEntry::provider_output(
            "session-1",
            "run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderTool,
            Some("tool-1".to_string()),
            r#"{"tool":"bash","status":"completed","input":{"command":"cargo test"}}"#,
        ),
        context,
    );
    store
        .append(&tool)
        .expect("promptless provider activity should append");

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 1, None).expect("outline should load");

    assert_eq!(outline.turns.len(), 1);
    assert_eq!(outline.turns[0].turn_id, "run-1");
    assert!(
        outline.turns[0]
            .user_prompt
            .entry
            .text
            .contains("no recorded prompt"),
        "{:?}",
        outline.turns[0].user_prompt
    );
    assert_eq!(outline.turns[0].blobs.len(), 1);
    assert_eq!(
        outline.turns[0].blobs[0].kind,
        SessionHistoryEntryKind::ProviderTool
    );
    assert_eq!(outline.turns[0].blobs[0].summary, "$ cargo test");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_pages_promptless_provider_activity_groups() {
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

    let latest = load_agent_outline(&store, "session-1", "agent-1", 2, None)
        .expect("latest outline should load");

    assert_eq!(latest.turns.len(), 2);
    assert_eq!(latest.turns[0].turn_id, "turn-2");
    assert_eq!(latest.turns[1].turn_id, "turn-3");
    assert_eq!(
        latest
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.before_sequence),
        Some(20)
    );

    let older = load_agent_outline(
        &store,
        "session-1",
        "agent-1",
        2,
        latest
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.before_sequence),
    )
    .expect("older promptless outline page should load");

    assert_eq!(older.turns.len(), 1);
    assert_eq!(older.turns[0].turn_id, "turn-1");
    assert_eq!(older.next_cursor, None);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn agent_outline_preserves_external_identity_for_promptless_observed_activity() {
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

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 1, None).expect("outline should load");

    assert_eq!(outline.turns.len(), 1);
    let turn = &outline.turns[0];
    assert_eq!(turn.prompt_origin, PromptOrigin::External);
    assert_eq!(turn.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        turn.external_provider_session_id.as_deref(),
        Some("thread-1")
    );
    assert_eq!(turn.external_provider_turn_id.as_deref(), Some("tool-1"));
    assert!(turn.user_prompt.entry.is_external_provider_observed());
    assert!(
        turn.user_prompt.entry.text.contains("no recorded prompt"),
        "{:?}",
        turn.user_prompt
    );
    assert_eq!(turn.blobs.len(), 1);
    assert_eq!(turn.blobs[0].kind, SessionHistoryEntryKind::ProviderTool);
    assert_eq!(turn.blobs[0].summary, "$ cargo test");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
