mod legacy_import;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};

use base64::Engine;

use crate::config::DaemonConfig;
use crate::session::{CreateSessionRequest, PromptAttachment, SessionService};
use crate::terminal::TerminalOutputKind;

use super::{
    HistoryEvent, HistoryEventKind, HistoryEventQuery, HistoryEventRole, HistoryEventTurnContext,
    OperationalHistoryStore, SessionHistoryEntry, SessionHistoryEntryKind,
    SessionHistoryEntrySource, SessionHistoryPromptAttachment, SessionHistoryStore,
};

fn external_observed_entry(session_id: &str) -> SessionHistoryEntry {
    SessionHistoryEntry::external_provider_observed(
        session_id,
        Some("external-observer:agent-1"),
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "external output",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(1_000),
    )
}

#[test]
fn session_history_entry_source_metadata_line_matches_serialized_source() {
    let serialized = serde_json::to_value(SessionHistoryEntrySource::ExternalProviderObserved)
        .expect("source should serialize");
    assert_eq!(serialized, serde_json::json!("external_provider_observed"));
    assert_eq!(
        SessionHistoryEntrySource::ExternalProviderObserved.metadata_line(),
        serialized
            .as_str()
            .expect("serialized source should be a string")
    );
    assert!(
        SessionHistoryEntrySource::metadata_text_contains_external_provider_observed(
            "merge-key\nexternal_provider_observed\nturn-id",
        )
    );
    assert!(
        !SessionHistoryEntrySource::metadata_text_contains_external_provider_observed(
            "merge-key\nexternal_provider_observed_extra",
        )
    );

    let observed = SessionHistoryEntry::external_provider_observed(
        "session-1",
        None,
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "external output",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(1_000),
    );
    assert!(observed.is_external_provider_observed());
    assert_eq!(
        observed.source_attachment_id.as_deref(),
        Some("external:codex")
    );
    assert_eq!(
        observed.external_provider_observed_turn_id(),
        Some("turn-1")
    );
    assert_eq!(
        observed.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );

    let arroba_owned =
        SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "arroba");
    assert!(!arroba_owned.is_external_provider_observed());
    assert_eq!(arroba_owned.external_provider_observed_turn_id(), None);
    assert_eq!(
        arroba_owned.prompt_origin,
        Some(crate::session::PromptOrigin::Arroba)
    );
}

#[test]
fn appends_and_loads_session_history() {
    let config = DaemonConfig::for_tests();
    let mut sessions = SessionService::new(&config);
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let store = SessionHistoryStore::new(config.session_history_root.clone())
        .expect("history store should initialize");

    store
        .append(
            &session,
            &SessionHistoryEntry::user_prompt(session.id(), "attachment-1", "agent-1", "hello\n"),
        )
        .expect("user prompt should persist");
    store
        .append(
            &session,
            &SessionHistoryEntry::provider_output(
                session.id(),
                "provider-run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                None,
                "world",
            ),
        )
        .expect("provider output should persist");

    let entries = store.load(&session).expect("history should load");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].kind, SessionHistoryEntryKind::UserPrompt);
    assert_eq!(
        entries[0].prompt_origin,
        Some(crate::session::PromptOrigin::Arroba)
    );
    assert_eq!(entries[1].kind, SessionHistoryEntryKind::ProviderOutput);
    assert_eq!(entries[1].prompt_origin, None);
}

#[test]
fn session_history_append_rejects_prompt_origin_without_source_attachment() {
    let config = DaemonConfig::for_tests();
    let mut sessions = SessionService::new(&config);
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let store = SessionHistoryStore::new(config.session_history_root.clone())
        .expect("history store should initialize");
    let entry = SessionHistoryEntry::provider_output(
        session.id(),
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("output-1".to_string()),
        "output",
    )
    .with_prompt_origin(crate::session::PromptOrigin::Arroba);

    let error = store
        .append(&session, &entry)
        .expect_err("prompt-owned history without source attachment should fail");

    assert!(
        error
            .to_string()
            .contains("prompt-owned history entry must include source attachment"),
        "{error}"
    );
}

#[test]
fn session_history_append_rejects_external_provider_observed_without_complete_identity() {
    let config = DaemonConfig::for_tests();
    let mut sessions = SessionService::new(&config);
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let store = SessionHistoryStore::new(config.session_history_root.clone())
        .expect("history store should initialize");
    let valid = external_observed_entry(session.id());

    let invalid_entries = [
        {
            let mut entry = valid.clone();
            entry.external_provider = None;
            (entry, "external provider")
        },
        {
            let mut entry = valid.clone();
            entry.external_provider_session_id = Some(" ".to_string());
            (entry, "external provider session id")
        },
        {
            let mut entry = valid.clone();
            entry.external_provider_turn_id = None;
            (entry, "external provider turn id")
        },
        {
            let mut entry = valid.clone();
            entry.merge_key = None;
            (entry, "external provider merge key")
        },
    ];
    for (entry, missing_field) in invalid_entries {
        let error = store
            .append(&session, &entry)
            .expect_err("external-observed history without complete identity should fail");
        assert!(
            error.to_string().contains(&format!(
                "external-observed history entry must include {missing_field}"
            )),
            "{error}"
        );
    }

    store
        .append(&session, &valid)
        .expect("complete external-observed history should append");
    let entries = store.load(&session).expect("history should load");
    assert_eq!(entries, vec![valid]);
}

#[test]
fn session_history_replace_rejects_external_provider_observed_without_complete_identity() {
    let config = DaemonConfig::for_tests();
    let mut sessions = SessionService::new(&config);
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let store = SessionHistoryStore::new(config.session_history_root.clone())
        .expect("history store should initialize");
    let valid = external_observed_entry(session.id());
    let merge_key = valid
        .merge_key
        .clone()
        .expect("external observed entry should have a merge key");
    store
        .append(&session, &valid)
        .expect("complete external-observed history should append");

    let mut invalid_replacement = valid.clone();
    invalid_replacement.text = "replacement".to_string();
    invalid_replacement.external_provider_turn_id = None;
    let error = store
        .replace_by_merge_key(&session, &merge_key, &invalid_replacement)
        .expect_err("external-observed replacement without complete identity should fail");
    assert!(
        error
            .to_string()
            .contains("external-observed history entry must include external provider turn id"),
        "{error}"
    );

    let entries = store.load(&session).expect("history should load");
    assert_eq!(entries, vec![valid]);
}

#[test]
fn session_history_replacement_appends_and_deduplicates_without_rewriting_the_file() {
    let config = DaemonConfig::for_tests();
    let mut sessions = SessionService::new(&config);
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let store = SessionHistoryStore::new(config.session_history_root.clone())
        .expect("history store should initialize");
    let original = external_observed_entry(session.id());
    let merge_key = original.merge_key.clone().expect("merge key");
    store
        .append(&session, &original)
        .expect("original history should append");
    let path = store.path_for_session(&session);
    let original_bytes = std::fs::metadata(&path).expect("history metadata").len();
    let mut replacement = original.clone();
    replacement.text = "replacement output".to_string();

    assert!(store
        .replace_by_merge_key(&session, &merge_key, &replacement)
        .expect("history replacement should append"));

    assert!(
        std::fs::metadata(&path).expect("history metadata").len() > original_bytes,
        "legacy replacement should be append-only"
    );
    assert_eq!(
        store.load(&session).expect("history should load"),
        vec![replacement]
    );
}

#[test]
fn operational_history_append_rejects_prompt_origin_without_source_attachment_before_sequence() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-prompt-owned-validation-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    let invalid = SessionHistoryEntry::provider_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("output-1".to_string()),
        "output",
    )
    .with_prompt_origin(crate::session::PromptOrigin::Arroba);

    let error = store
        .append_transcript(&invalid, HistoryEventTurnContext::default())
        .expect_err("prompt-owned operational history without source attachment should fail");

    assert!(
        error
            .to_string()
            .contains("prompt-owned history entry must include source attachment"),
        "{error}"
    );

    let valid = invalid.with_source_attachment_id(Some("attachment-1".to_string()));
    let event = store
        .append_transcript(&valid, HistoryEventTurnContext::default())
        .expect("valid prompt-owned operational history should append");
    assert_eq!(event.sequence, 1);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_append_rejects_external_provider_observed_without_complete_identity_before_sequence(
) {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-external-observed-validation-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    let valid = external_observed_entry("session-1");
    let mut invalid = valid.clone();
    invalid.merge_key = Some(" ".to_string());

    let error = store
        .append_transcript(&invalid, HistoryEventTurnContext::default())
        .expect_err("external-observed operational history without complete identity should fail");
    assert!(
        error
            .to_string()
            .contains("external-observed history entry must include external provider merge key"),
        "{error}"
    );

    let event = store
        .append_transcript(&valid, HistoryEventTurnContext::default())
        .expect("complete external-observed operational history should append");
    assert_eq!(event.sequence, 1);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn session_history_load_hides_external_observer_state_signals() {
    let config = DaemonConfig::for_tests();
    let mut sessions = SessionService::new(&config);
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let store = SessionHistoryStore::new(config.session_history_root.clone())
        .expect("history store should initialize");
    let signal = SessionHistoryEntry::external_provider_observed_state_signal(
        session.id(),
        Some("run-1"),
        "agent-1",
        "codex",
        "thread-1",
        crate::history::EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
        "external:codex:thread-1:item-1",
        "item-1".to_string(),
        Some(2_000),
    );

    store
        .append(&session, &signal)
        .expect("state signal should persist");

    let entries = store.load(&session).expect("history should load");
    assert!(
        entries.is_empty(),
        "internal external-observer state signals must not render as transcript entries"
    );
}

#[test]
fn session_history_load_rehydrates_file_image_attachment_previews() {
    let config = DaemonConfig::for_tests();
    let mut sessions = SessionService::new(&config);
    let session = sessions
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let store = SessionHistoryStore::new(config.session_history_root.clone())
        .expect("history store should initialize");
    let image_path = std::env::temp_dir().join(format!(
        "arroba-jsonl-history-preview-{}-{}.png",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    std::fs::write(&image_path, b"file-image").expect("fixture image should write");
    let mut entry =
        SessionHistoryEntry::user_prompt(session.id(), "attachment-1", "agent-1", "inspect");
    entry.attachments = vec![SessionHistoryPromptAttachment {
        url: format!("file://{}", image_path.display()),
        mime: "image/png".to_string(),
        filename: Some("file-screenshot.png".to_string()),
        preview_url: None,
    }];

    store
        .append(&session, &entry)
        .expect("entry should persist");
    let entries = store.load(&session).expect("history should load");

    assert_eq!(
        entries[0]
            .attachments
            .first()
            .and_then(|attachment| attachment.preview_url.as_deref()),
        Some("data:image/png;base64,ZmlsZS1pbWFnZQ==")
    );

    let _ = std::fs::remove_file(image_path);
}

#[test]
fn converts_session_history_entry_to_canonical_history_event() {
    let entry = SessionHistoryEntry::provider_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderTool,
        Some("tool:browser".to_string()),
        "called browser",
    )
    .with_prompt_origin(crate::session::PromptOrigin::External);
    let event = HistoryEvent::transcript(
        7,
        &entry,
        HistoryEventTurnContext {
            provider: Some("codex".to_string()),
            model: Some("gpt-5.2".to_string()),
            turn_id: Some("turn-1".to_string()),
            prompt_id: Some("prompt-1".to_string()),
            worktree_path: Some("/repo".to_string()),
            ..HistoryEventTurnContext::default()
        },
    );

    assert_eq!(event.event_id, format!("evt_{}_7", entry.timestamp_ms));
    assert_eq!(event.sequence, 7);
    assert_eq!(event.session_id.as_deref(), Some("session-1"));
    assert_eq!(event.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(event.provider.as_deref(), Some("codex"));
    assert_eq!(event.model.as_deref(), Some("gpt-5.2"));
    assert_eq!(event.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(event.prompt_id.as_deref(), Some("prompt-1"));
    assert_eq!(event.provider_run_id.as_deref(), Some("provider-run-1"));
    assert_eq!(event.worktree_path.as_deref(), Some("/repo"));
    assert_eq!(event.kind, HistoryEventKind::ProviderTool);
    assert_eq!(event.role, Some(HistoryEventRole::Tool));
    assert_eq!(event.content.as_deref(), Some("called browser"));
    assert_eq!(
        event
            .metadata
            .get("merge_key")
            .and_then(|value| value.as_str()),
        Some("tool:browser")
    );
    assert_eq!(
        event
            .metadata
            .get("prompt_origin")
            .and_then(|value| value.as_str()),
        Some("external")
    );
    let round_tripped = event
        .to_session_history_entry()
        .expect("transcript event should convert back");
    assert_eq!(round_tripped.kind, SessionHistoryEntryKind::ProviderTool);
    assert_eq!(round_tripped.text, "called browser");
    assert_eq!(round_tripped.merge_key.as_deref(), Some("tool:browser"));
    assert_eq!(
        round_tripped.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
}

#[test]
fn operational_history_replaces_transcripts_through_merge_key_index() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-indexed-replace-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    let original = SessionHistoryEntry::provider_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("provider-turn-1".to_string()),
        "partial",
    );
    let event = store
        .append_transcript(&original, HistoryEventTurnContext::default())
        .expect("original transcript should append");
    let replacement = SessionHistoryEntry::provider_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("provider-turn-1".to_string()),
        "complete",
    );
    let replaced = store
        .replace_transcript_by_merge_key(
            "session-1",
            Some("agent-1"),
            "provider-turn-1",
            &replacement,
            HistoryEventTurnContext::default(),
        )
        .expect("indexed replacement should succeed")
        .expect("indexed transcript should exist");
    assert_eq!(replaced.event_id, event.event_id);
    assert_eq!(replaced.sequence, event.sequence);
    assert_eq!(replaced.content.as_deref(), Some("complete"));

    let connection = store.connection.lock().expect("history lock should hold");
    let plan = connection
        .query_row(
            "EXPLAIN QUERY PLAN SELECT event_json FROM history_events
             WHERE session_id = ?1 AND agent_id IS ?2 AND merge_key = ?3
             ORDER BY sequence ASC LIMIT 1",
            rusqlite::params!["session-1", "agent-1", "provider-turn-1"],
            |row| row.get::<_, String>(3),
        )
        .expect("replacement query plan should load");
    assert!(
        plan.contains("idx_history_events_session_agent_merge_sequence"),
        "replacement lookup should use the merge-key index: {plan}"
    );
    drop(connection);
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_reads_do_not_hold_the_writer_connection() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-read-write-isolation-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    let read_connection = store
        .lock_read_connection(Some("session-1"))
        .expect("read connection should lock");
    let entry = SessionHistoryEntry::provider_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        "output while hydration holds a reader",
    );
    store
        .append_transcript(&entry, HistoryEventTurnContext::default())
        .expect("writer should remain independent from hydration readers");
    drop(read_connection);
    assert!(store
        .has_session_events("session-1")
        .expect("session history should remain readable"));
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn canonical_history_events_preserve_prompt_attachments() {
    let image_path = std::env::temp_dir().join(format!(
        "arroba-history-preview-{}-{}.png",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    std::fs::write(&image_path, b"file-image").expect("fixture image should write");
    let contents_base64 = base64::engine::general_purpose::STANDARD.encode("image");
    let inline_attachment = PromptAttachment::new(
        "arroba-terminal://prompt-attachment/attachment-1/screenshot.png",
        "image/png",
        Some("screenshot.png".to_string()),
    )
    .with_contents_base64(contents_base64);
    let file_attachment = PromptAttachment::new(
        format!("file://{}", image_path.display()),
        "image/png",
        Some("file-screenshot.png".to_string()),
    );
    let entry = SessionHistoryEntry::user_prompt_with_attachments(
        "session-1",
        "attachment-1",
        "agent-1",
        "inspect",
        &[inline_attachment, file_attachment],
    );
    let event = HistoryEvent::transcript(7, &entry, HistoryEventTurnContext::default());
    let round_tripped = event
        .to_session_history_entry()
        .expect("transcript event should convert back");
    let attachment = round_tripped
        .attachments
        .first()
        .expect("attachment should round-trip through operational history");

    assert_eq!(attachment.filename.as_deref(), Some("screenshot.png"));
    assert_eq!(attachment.mime, "image/png");
    assert_eq!(
        attachment.preview_url.as_deref(),
        Some("data:image/png;base64,aW1hZ2U=")
    );
    assert_eq!(
        round_tripped
            .attachments
            .get(1)
            .and_then(|attachment| attachment.preview_url.as_deref()),
        Some("data:image/png;base64,ZmlsZS1pbWFnZQ==")
    );

    let _ = std::fs::remove_file(image_path);
}

#[test]
fn canonical_history_events_rehydrate_file_image_attachment_previews() {
    let image_path = std::env::temp_dir().join(format!(
        "arroba-operational-history-preview-{}-{}.png",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    std::fs::write(&image_path, b"file-image").expect("fixture image should write");
    let mut entry =
        SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "inspect");
    entry.attachments = vec![SessionHistoryPromptAttachment {
        url: format!("file://{}", image_path.display()),
        mime: "image/png".to_string(),
        filename: Some("file-screenshot.png".to_string()),
        preview_url: None,
    }];
    let event = HistoryEvent::transcript(7, &entry, HistoryEventTurnContext::default());
    let round_tripped = event
        .to_session_history_entry()
        .expect("transcript event should convert back");

    assert_eq!(
        round_tripped
            .attachments
            .first()
            .and_then(|attachment| attachment.preview_url.as_deref()),
        Some("data:image/png;base64,ZmlsZS1pbWFnZQ==")
    );

    let _ = std::fs::remove_file(image_path);
}

#[test]
fn operational_history_store_reports_max_prompt_number() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-max-prompt-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    for (sequence, prompt_id) in [(1, "prompt-2"), (2, "manual-99"), (3, "prompt-17")] {
        let entry = SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hi");
        let event = HistoryEvent::transcript(
            sequence,
            &entry,
            HistoryEventTurnContext {
                prompt_id: Some(prompt_id.to_string()),
                ..HistoryEventTurnContext::default()
            },
        );
        store
            .append(&event)
            .expect("event should append to operational history");
    }

    assert_eq!(
        store
            .max_prompt_number()
            .expect("max prompt number should load"),
        17
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_store_excludes_external_observed_prompts_from_arroba_owned_prompts() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-arroba-prompts-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    let arroba_prompt =
        SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "arroba");
    let external_origin_prompt =
        SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "external origin")
            .with_prompt_origin(crate::session::PromptOrigin::External);
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        None,
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "external",
        "codex",
        "thread-1",
        Some("turn-1".to_string()),
        Some(2_000),
    );
    for (sequence, entry) in [
        (1, arroba_prompt),
        (2, external_origin_prompt),
        (3, external_prompt),
    ] {
        store
            .append(&HistoryEvent::transcript(
                sequence,
                &entry,
                HistoryEventTurnContext {
                    turn_id: Some(format!("turn-{sequence}")),
                    prompt_id: Some(format!("prompt-{sequence}")),
                    ..HistoryEventTurnContext::default()
                },
            ))
            .expect("prompt event should append");
    }

    assert_eq!(
        store
            .load_arroba_owned_prompt_texts("session-1", "agent-1")
            .expect("arroba-owned prompts should load"),
        vec!["arroba".to_string()]
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_store_indexes_external_import_history() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-external-index-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    let arroba_prompt =
        SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "arroba");
    let external_origin_prompt =
        SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "external origin")
            .with_prompt_origin(crate::session::PromptOrigin::External);
    let external_prompt = SessionHistoryEntry::external_provider_observed_with_merge_key(
        "session-1",
        None,
        "agent-1",
        SessionHistoryEntryKind::UserPrompt,
        "external prompt",
        "codex",
        "thread-1",
        Some("external:codex:thread-1:turn-1:prompt".to_string()),
        Some("turn-1".to_string()),
        Some(2_000),
    );
    let external_output = SessionHistoryEntry::external_provider_observed_with_merge_key(
        "session-1",
        None,
        "agent-1",
        SessionHistoryEntryKind::ProviderOutput,
        "external output",
        "codex",
        "thread-1",
        Some("external:codex:thread-1:turn-1:output".to_string()),
        Some("turn-1".to_string()),
        Some(2_100),
    );
    let mut external_status = SessionHistoryEntry::external_provider_observed_with_merge_key(
        "session-1",
        None,
        "agent-1",
        SessionHistoryEntryKind::ProviderStatus,
        "external settled",
        "codex",
        "thread-1",
        Some("external:codex:thread-1:turn-1:status".to_string()),
        Some("turn-1".to_string()),
        Some(2_200),
    );
    external_status.external_observation =
        Some(crate::history::SessionHistoryExternalObservation::active_prompt_settled());
    let external_state_signal = SessionHistoryEntry::external_provider_observed_state_signal(
        "session-1",
        None,
        "agent-1",
        "codex",
        "thread-1",
        crate::history::EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
        "external:codex:thread-1:turn-1:status",
        "active_prompt_settled".to_string(),
        Some(2_300),
    );
    let state_signal_merge_key = external_state_signal
        .merge_key
        .clone()
        .expect("state signal should have merge key");
    assert_eq!(
        HistoryEvent::transcript(4, &external_status, HistoryEventTurnContext::default())
            .session_history_external_observation(),
        Some(crate::history::SessionHistoryExternalObservation::active_prompt_settled())
    );

    for (sequence, entry) in [
        (1, arroba_prompt),
        (2, external_origin_prompt),
        (3, external_prompt),
        (4, external_output),
        (5, external_status),
        (6, external_state_signal),
    ] {
        store
            .append(&HistoryEvent::transcript(
                sequence,
                &entry,
                HistoryEventTurnContext {
                    turn_id: entry
                        .external_provider_observed_turn_id()
                        .map(str::to_string),
                    prompt_id: Some(format!("prompt-{sequence}")),
                    ..HistoryEventTurnContext::default()
                },
            ))
            .expect("history event should append");
    }

    let index = store
        .load_external_import_history_index("session-1", "agent-1", "external:codex:thread-1")
        .expect("external import index should load");
    assert_eq!(index.arroba_owned_prompts, vec!["arroba".to_string()]);
    assert_eq!(
        index
            .external_entries_by_merge_key
            .get("external:codex:thread-1:turn-1:prompt")
            .map(|entry| (entry.kind, entry.text.as_str())),
        Some((SessionHistoryEntryKind::UserPrompt, "external prompt"))
    );
    let external_prompt_entry = index
        .external_entries_by_merge_key
        .get("external:codex:thread-1:turn-1:prompt")
        .expect("external prompt should be indexed");
    assert_eq!(
        external_prompt_entry.external_provider.as_deref(),
        Some("codex")
    );
    assert_eq!(
        external_prompt_entry
            .external_provider_session_id
            .as_deref(),
        Some("thread-1")
    );
    assert_eq!(
        external_prompt_entry.external_provider_turn_id.as_deref(),
        Some("turn-1")
    );
    assert_eq!(external_prompt_entry.observed_at_ms, Some(2_000));
    assert_eq!(
        index
            .external_entries_by_merge_key
            .get("external:codex:thread-1:turn-1:output")
            .map(|entry| (entry.kind, entry.text.as_str())),
        Some((SessionHistoryEntryKind::ProviderOutput, "external output"))
    );
    assert_eq!(
        index
            .external_entries_by_merge_key
            .get("external:codex:thread-1:turn-1:status")
            .and_then(|entry| entry.external_observation.as_ref())
            .map(|observation| observation.settles_active_prompt),
        Some(true)
    );
    assert!(
        !index
            .external_entries_by_merge_key
            .contains_key(&state_signal_merge_key),
        "internal external-observer state signals must not pollute the provider transcript index"
    );
    let visible_entries = store
        .load_session_history_entries("session-1", Some("agent-1"))
        .expect("session history entries should load");
    assert!(
        visible_entries
            .iter()
            .all(|entry| entry.merge_key.as_ref() != Some(&state_signal_merge_key)),
        "internal external-observer state signals must not render as transcript entries"
    );
    assert!(
        store
            .load_session_events("session-1", Some("agent-1"))
            .expect("raw operational events should load")
            .iter()
            .filter_map(|event| event.to_session_history_entry())
            .any(|entry| entry.merge_key.as_ref() == Some(&state_signal_merge_key)),
        "raw operational events must retain state signals for lifecycle projection"
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_lists_agents_by_first_event_sequence() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-agent-order-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    for (sequence, agent_id, prompt) in [
        (20, "agent-1", "created last"),
        (10, "agent-10", "created first"),
        (15, "agent-2", "created second"),
    ] {
        let entry = SessionHistoryEntry::user_prompt("session-1", "attachment-1", agent_id, prompt);
        store
            .append(&HistoryEvent::transcript(
                sequence,
                &entry,
                HistoryEventTurnContext {
                    turn_id: Some(format!("{agent_id}-turn")),
                    prompt_id: Some(format!("{agent_id}-prompt")),
                    ..HistoryEventTurnContext::default()
                },
            ))
            .expect("agent history event should append");
    }

    assert_eq!(
        store
            .list_session_history_agent_ids("session-1")
            .expect("history agent ids should load"),
        vec![
            "agent-10".to_string(),
            "agent-2".to_string(),
            "agent-1".to_string()
        ]
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_external_import_index_uses_latest_duplicate_merge_key() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-external-index-duplicate-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    let merge_key = "external:codex:thread-1:assistant-1";
    for (sequence, text, settles_active_prompt) in [
        (20, "old assistant snapshot", false),
        (30, "new assistant snapshot", true),
    ] {
        let mut entry = SessionHistoryEntry::external_provider_observed_with_merge_key(
            "session-1",
            None,
            "agent-1",
            SessionHistoryEntryKind::ProviderOutput,
            text,
            "codex",
            "thread-1",
            Some(merge_key.to_string()),
            Some("assistant-1".to_string()),
            Some(sequence * 100),
        );
        entry.external_observation = settles_active_prompt
            .then(crate::history::SessionHistoryExternalObservation::active_prompt_settled);
        store
            .append(&HistoryEvent::transcript(
                sequence,
                &entry,
                HistoryEventTurnContext {
                    turn_id: Some("assistant-1".to_string()),
                    prompt_id: Some("external:codex:thread-1:user-1".to_string()),
                    ..HistoryEventTurnContext::default()
                },
            ))
            .expect("duplicate external event should append");
    }

    let index = store
        .load_external_import_history_index("session-1", "agent-1", "external:codex:thread-1")
        .expect("external import index should load");

    let latest = index
        .external_entries_by_merge_key
        .get(merge_key)
        .expect("duplicate merge key should be indexed");
    assert_eq!(latest.text, "new assistant snapshot");
    assert_eq!(latest.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        latest.external_provider_session_id.as_deref(),
        Some("thread-1")
    );
    assert_eq!(
        latest.external_provider_turn_id.as_deref(),
        Some("assistant-1")
    );
    assert_eq!(latest.observed_at_ms, Some(3_000));
    assert_eq!(
        latest
            .external_observation
            .as_ref()
            .map(|observation| observation.settles_active_prompt),
        Some(true)
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_store_appends_and_loads_events_idempotently() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let entry = SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "hi");
    let event = HistoryEvent::transcript(
        1,
        &entry,
        HistoryEventTurnContext {
            provider: Some("opencode".to_string()),
            model: Some("gpt-5.2".to_string()),
            ..HistoryEventTurnContext::default()
        },
    );

    store
        .append(&event)
        .expect("event should append to operational history");
    store
        .append(&event)
        .expect("duplicate event should be ignored");

    let all_events = store
        .load_session_events("session-1", None)
        .expect("session events should load");
    assert_eq!(all_events.len(), 1);
    assert_eq!(all_events[0].event_id, event.event_id);
    assert_eq!(all_events[0].kind, HistoryEventKind::UserPrompt);
    assert_eq!(all_events[0].provider.as_deref(), Some("opencode"));

    let agent_events = store
        .load_session_events("session-1", Some("agent-1"))
        .expect("agent events should load");
    assert_eq!(agent_events.len(), 1);

    let queried = store
        .query_events(HistoryEventQuery {
            session_id: Some("session-1".to_string()),
            provider: Some("opencode".to_string()),
            text: Some("hi".to_string()),
            limit: Some(10),
            ..HistoryEventQuery::default()
        })
        .expect("query should load matching events");
    assert_eq!(queried.len(), 1);
    assert_eq!(queried[0].event_id, event.event_id);

    store
        .enqueue_archive_events(std::slice::from_ref(&event))
        .expect("event should enqueue for archive");
    store
        .enqueue_archive_events(std::slice::from_ref(&event))
        .expect("duplicate outbox event should refresh pending payload");
    let pending = store
        .load_pending_archive_events(10)
        .expect("pending outbox events should load");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event.event_id, event.event_id);
    assert_eq!(pending[0].attempts, 0);

    let mut replacement_event = event.clone();
    replacement_event.timestamp_ms += 1;
    replacement_event.event_id = event.event_id.clone();
    replacement_event.content = Some("final observed content".to_string());
    store
        .mark_archive_events_failed(std::slice::from_ref(&event.event_id), "adapter down")
        .expect("outbox failure should record before replacement");
    store
        .enqueue_archive_events(std::slice::from_ref(&replacement_event))
        .expect("replacement event should refresh pending outbox payload");
    let pending_after_replacement = store
        .load_pending_archive_events(10)
        .expect("pending replacement outbox event should load");
    assert_eq!(pending_after_replacement.len(), 1);
    assert_eq!(pending_after_replacement[0].event.event_id, event.event_id);
    assert_eq!(
        pending_after_replacement[0].event.content.as_deref(),
        Some("final observed content")
    );
    assert_eq!(pending_after_replacement[0].attempts, 0);
    assert_eq!(pending_after_replacement[0].last_error, None);

    drop(store);
    let store = OperationalHistoryStore::open(path.clone())
        .expect("operational history store should reopen");
    let pending_after_reopen = store
        .load_pending_archive_events(10)
        .expect("pending outbox should survive reopen");
    assert_eq!(pending_after_reopen.len(), 1);
    store
        .mark_archive_events_failed(std::slice::from_ref(&event.event_id), "adapter down")
        .expect("outbox failure should record");
    let pending_after_failure = store
        .load_pending_archive_events(10)
        .expect("failed outbox item should remain pending");
    assert_eq!(pending_after_failure[0].attempts, 1);
    assert_eq!(
        pending_after_failure[0].last_error.as_deref(),
        Some("adapter down")
    );
    store
        .mark_archive_events_accepted(std::slice::from_ref(&event.event_id))
        .expect("outbox acceptance should record");
    assert!(
        store
            .load_pending_archive_events(10)
            .expect("pending outbox should load")
            .is_empty(),
        "accepted outbox item should stop being pending"
    );
    let deleted = store
        .prune_events_before(i64::MAX as u64, false)
        .expect("archived events should prune");
    assert_eq!(deleted, 1);
    assert!(!store
        .has_session_events("session-1")
        .expect("session event presence should load"));
    assert!(
        store
            .legacy_fallback_disabled("session-1")
            .expect("legacy fallback marker should load"),
        "pruned sessions should not fall back to legacy JSONL"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_amortizes_size_budget_checks_for_small_appends() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-amortized-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store = OperationalHistoryStore::open_with_read_delay_and_max_size(
        path.clone(),
        0,
        16 * 1024 * 1024,
    )
    .expect("operational history store should open");
    let entry = SessionHistoryEntry::provider_output(
        "session-amortized",
        "provider-run-amortized",
        Some("agent-amortized"),
        TerminalOutputKind::ProviderOutput,
        Some("chunk-1".to_string()),
        "small output",
    );
    let event = HistoryEvent::transcript(1, &entry, HistoryEventTurnContext::default());
    store.append(&event).expect("event should append");
    assert!(
        store
            .appended_bytes_since_size_check
            .load(Ordering::Acquire)
            > 0,
        "small appends should defer retention work until the byte threshold is reached"
    );

    store
        .enforce_size_budget()
        .expect("explicit retention check should succeed");
    assert_eq!(
        store
            .appended_bytes_since_size_check
            .load(Ordering::Acquire),
        0
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_writer_groups_concurrent_acknowledged_appends() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-grouped-writes-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let writers = 32;
    let barrier = Arc::new(Barrier::new(writers));
    let mut threads = Vec::new();
    for index in 0..writers {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            store
                .append_operational_event(
                    HistoryEventKind::Notice,
                    Some(HistoryEventRole::System),
                    Some(format!("event-{index}")),
                    Default::default(),
                    HistoryEventTurnContext::default(),
                )
                .expect("concurrent history event should append");
        }));
    }
    for thread in threads {
        thread.join().expect("history append thread should join");
    }

    let health = store.writer_health_snapshot();
    assert_eq!(health.committed_records, writers as u64);
    assert!(health.committed_batches < writers as u64, "{health:?}");
    assert!(health.max_batch_records > 1, "{health:?}");
    assert_eq!(
        store
            .query_events(HistoryEventQuery::default())
            .expect("written history should query")
            .len(),
        writers
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn operational_history_enforces_size_budget_for_temp_stores() {
    let path = std::env::temp_dir().join(format!(
        "arroba-operational-history-cap-{}-{}.db",
        std::process::id(),
        super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let max_size_bytes = 1024 * 1024;
    let store =
        OperationalHistoryStore::open_with_read_delay_and_max_size(path.clone(), 0, max_size_bytes)
            .expect("operational history store should open");
    for index in 0..80 {
        let entry = SessionHistoryEntry::provider_output(
            "session-cap",
            "provider-run-cap",
            Some("agent-cap"),
            TerminalOutputKind::ProviderOutput,
            Some(format!("chunk-{index}")),
            &"x".repeat(64 * 1024),
        );
        let event = HistoryEvent::transcript(index + 1, &entry, HistoryEventTurnContext::default());
        store.append(&event).expect("event should append");
    }

    let size = std::fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        + std::fs::metadata(path.with_extension("db-wal"))
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    assert!(
        size <= max_size_bytes,
        "operational history should stay under the configured hard cap: {size}"
    );
    assert!(
        store
            .load_session_events("session-cap", None)
            .expect("events should load")
            .len()
            < 80,
        "oldest events should be pruned once the size cap is exceeded"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
