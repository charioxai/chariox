use crate::history::{HistoryEventTurnContext, OperationalHistoryStore, SessionHistoryEntry};
use crate::terminal::TerminalOutputKind;

#[test]
fn operational_history_imports_missing_legacy_transcripts_idempotently() {
    let path = std::env::temp_dir().join(format!(
        "chariox-operational-history-legacy-import-{}-{}.db",
        std::process::id(),
        super::super::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history should open");
    let external_prompt = SessionHistoryEntry::external_provider_observed(
        "session-1",
        None,
        "agent-1",
        crate::history::SessionHistoryEntryKind::UserPrompt,
        "legacy prompt",
        "codex",
        "thread-1",
        Some("external-1".to_string()),
        Some(1),
    );
    store
        .append_transcripts(vec![(&external_prompt, HistoryEventTurnContext::default())])
        .expect("external prompt should append");
    assert!(
        store
            .load_chariox_owned_prompt_texts("session-1", "agent-1")
            .expect("chariox-owned prompt index should load")
            .is_empty(),
        "external-observed prompts should not count as Chariox-owned prompt text"
    );

    let mut legacy_prompt =
        SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "legacy prompt")
            .with_prompt_origin(crate::session::PromptOrigin::Chariox);
    legacy_prompt.merge_key = Some("prompt:legacy-1".to_string());
    let legacy_output = SessionHistoryEntry::provider_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        "legacy output",
    );

    let imported = store
        .append_missing_legacy_transcripts(&[legacy_prompt.clone(), legacy_output.clone()])
        .expect("legacy transcripts should import");
    assert_eq!(imported.len(), 2);
    assert_eq!(
        store
            .load_chariox_owned_prompt_texts("session-1", "agent-1")
            .expect("chariox owned prompt should load"),
        vec!["legacy prompt".to_string()]
    );

    let imported_again = store
        .append_missing_legacy_transcripts(&[legacy_prompt, legacy_output])
        .expect("legacy transcripts should import idempotently");
    assert!(imported_again.is_empty());
    assert_eq!(
        store
            .load_session_events("session-1", Some("agent-1"))
            .expect("session events should load")
            .len(),
        3
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
