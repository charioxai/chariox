use super::super::{
    WorkspaceLiveSyncApplyStatus, WorkspaceLiveSyncJournal, WorkspaceLiveSyncJournalEntry,
    WorkspaceLiveSyncPathApplyResult, WorkspaceLiveSyncTargetResult,
};
use super::support::workspace_live_sync_test_change;

#[test]
fn workspace_live_sync_journal_assigns_ordered_sequences_per_link() {
    let journal = WorkspaceLiveSyncJournal::default();
    let change = || workspace_live_sync_test_change("session-1");

    let first = journal.append_for_link("link-a", "shared-a", change());
    let second = journal.append_for_link("link-a", "shared-a", change());
    let other_link = journal.append_for_link("link-b", "shared-b", change());

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(other_link.sequence, 1);
    let entries = journal.entries_for_session("session-1");
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.link_id.as_str(), entry.sequence))
            .collect::<Vec<_>>(),
        vec![("link-a", 1), ("link-a", 2), ("link-b", 1)]
    );
}

#[test]
fn workspace_live_sync_journal_restores_durable_events_and_next_sequence() {
    let path = std::env::temp_dir().join(format!(
        "arroba-workspace-live-sync-journal-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let store =
        crate::durable_state::DurableKernelStateStore::open(path.clone()).expect("open store");

    let entry = WorkspaceLiveSyncJournalEntry {
        sequence: 1,
        link_id: "link-a".to_string(),
        link_name: "shared-a".to_string(),
        change: workspace_live_sync_test_change("session-1"),
    };
    let target_result = WorkspaceLiveSyncTargetResult {
        session_id: "session-1".to_string(),
        link_id: "link-a".to_string(),
        link_name: "shared-a".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/repo".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-2".to_string(),
        target_repo_root: "/target".to_string(),
        path_results: vec![WorkspaceLiveSyncPathApplyResult {
            path: "src/lib.rs".to_string(),
            status: WorkspaceLiveSyncApplyStatus::Applied,
            message: "applied cleanly".to_string(),
        }],
    };
    store
        .append_event(
            "workspace_live_sync.change_recorded",
            Some("session-1".to_string()),
            serde_json::json!({ "entry": entry }),
        )
        .expect("change event should append");
    store
        .append_event(
            "workspace_live_sync.target_results_recorded",
            Some("session-1".to_string()),
            serde_json::json!({ "target_results": [target_result] }),
        )
        .expect("target result event should append");
    store
        .append_event(
            "session.updated",
            Some("session-1".to_string()),
            serde_json::json!({ "unrelated": "x".repeat(1_000_000) }),
        )
        .expect("unrelated event should append");

    let journal =
        WorkspaceLiveSyncJournal::restore_from_durable_state(&store).expect("restore journal");

    assert_eq!(journal.entries_for_session("session-1").len(), 1);
    assert_eq!(journal.target_results_for_session("session-1").len(), 1);
    let next = journal.append_for_link(
        "link-a",
        "shared-a",
        workspace_live_sync_test_change("session-1"),
    );
    assert_eq!(next.sequence, 2);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
