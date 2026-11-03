use super::super::{
    CompletedGitTurnSnapshot, CompletedGitTurnSnapshotStore, CompletedTurnSettlementStatus,
    GitTurnSnapshotStore,
};
use super::support::tracked_snapshot;

#[test]
fn pending_turn_snapshot_lookup_does_not_consume_snapshot() {
    let snapshots = GitTurnSnapshotStore::default();
    let snapshot = tracked_snapshot(false, "");
    snapshots.insert(snapshot.clone());

    assert_eq!(
        snapshots.get("provider-run-1", "prompt-1"),
        Some(snapshot.clone())
    );
    assert_eq!(
        snapshots.get_for_provider_run("provider-run-1"),
        Some(snapshot.clone())
    );
    assert_eq!(
        snapshots.remove("provider-run-1", "prompt-1"),
        Some(snapshot)
    );
    assert_eq!(snapshots.get_for_provider_run("provider-run-1"), None);
}

#[test]
fn queued_prompt_snapshot_does_not_replace_completed_prompt_snapshot() {
    let snapshots = GitTurnSnapshotStore::default();
    let completed_prompt = tracked_snapshot(false, "");
    let mut promoted_prompt = completed_prompt.clone();
    promoted_prompt.prompt_id = "prompt-2".to_string();
    promoted_prompt.turn_id = "prompt-2".to_string();

    snapshots.insert(completed_prompt.clone());
    snapshots.insert(promoted_prompt.clone());

    assert_eq!(
        snapshots.get("provider-run-1", "prompt-1"),
        Some(completed_prompt.clone())
    );
    assert_eq!(
        snapshots.get("provider-run-1", "prompt-2"),
        Some(promoted_prompt.clone())
    );
    assert_eq!(
        snapshots.remove("provider-run-1", "prompt-1"),
        Some(completed_prompt)
    );
    assert_eq!(
        snapshots.get("provider-run-1", "prompt-2"),
        Some(promoted_prompt)
    );
}

#[test]
fn prompt_settlement_remains_latest_when_git_observation_is_missing() {
    let completed = CompletedGitTurnSnapshotStore::default();
    let mut first = tracked_snapshot(false, "");
    first.started_at_ms = Some(100);
    completed.record(CompletedGitTurnSnapshot::new(
        first.clone(),
        first,
        None,
        200,
    ));

    let prompt = crate::session::PromptQueueItem::new(
        "prompt-2",
        "workflow-run:run-1",
        "agent-1",
        "second workflow turn",
        crate::session::PromptStatus::Running,
    );
    completed.record_prompt_settlement(
        "session-1",
        "agent-1",
        "provider-run-1",
        &prompt,
        400,
        Some(300),
        CompletedTurnSettlementStatus::Completed,
    );

    let projection = completed
        .latest_projection_for_agent("session-1", "agent-1")
        .expect("settled prompt should be projected");
    assert_eq!(projection.turn_id, "prompt-2");
    assert_eq!(projection.completed_at_ms, 400);
    assert!(!projection.undo_available);
    assert_eq!(
        projection.undo_unavailable_reason.as_deref(),
        Some("workspace change observation is not available for this turn")
    );
}

#[test]
fn matching_git_observation_enriches_settled_prompt_projection() {
    let completed = CompletedGitTurnSnapshotStore::default();
    let prompt = crate::session::PromptQueueItem::new(
        "prompt-2",
        "attachment-1",
        "agent-1",
        "second turn",
        crate::session::PromptStatus::Running,
    );
    completed.record_prompt_settlement(
        "session-1",
        "agent-1",
        "provider-run-1",
        &prompt,
        400,
        Some(300),
        CompletedTurnSettlementStatus::Completed,
    );

    let mut observed = tracked_snapshot(false, "");
    observed.prompt_id = "prompt-2".to_string();
    observed.turn_id = "prompt-2".to_string();
    observed.started_at_ms = Some(300);
    completed.record(CompletedGitTurnSnapshot::new(
        observed.clone(),
        observed,
        None,
        410,
    ));

    let projection = completed
        .latest_projection_for_agent("session-1", "agent-1")
        .expect("observed prompt should be projected");
    assert_eq!(projection.turn_id, "prompt-2");
    assert!(projection.undo_available);
    assert_eq!(projection.undo_unavailable_reason, None);
}

#[test]
fn cancelled_settlement_survives_late_provider_completion_projection() {
    let completed = CompletedGitTurnSnapshotStore::default();
    let prompt = crate::session::PromptQueueItem::new(
        "prompt-cancelled",
        "attachment-1",
        "agent-1",
        "cancel me",
        crate::session::PromptStatus::Running,
    );
    completed.record_prompt_settlement(
        "session-1",
        "agent-1",
        "provider-run-1",
        &prompt,
        400,
        Some(300),
        CompletedTurnSettlementStatus::Cancelled,
    );
    completed.record_prompt_settlement(
        "session-1",
        "agent-1",
        "provider-run-1",
        &prompt,
        450,
        Some(300),
        CompletedTurnSettlementStatus::Completed,
    );

    let projection = completed
        .latest_projection_for_agent("session-1", "agent-1")
        .expect("cancelled turn should remain projected");
    assert_eq!(
        projection.settlement_status,
        CompletedTurnSettlementStatus::Cancelled
    );
}

#[test]
fn failed_settlement_enriches_matching_git_observation_by_prompt_identity() {
    let completed = CompletedGitTurnSnapshotStore::default();
    let prompt = crate::session::PromptQueueItem::new(
        "prompt-failed",
        "attachment-1",
        "agent-1",
        "fail me",
        crate::session::PromptStatus::Running,
    );
    completed.record_prompt_settlement(
        "session-1",
        "agent-1",
        "provider-run-1",
        &prompt,
        400,
        Some(300),
        CompletedTurnSettlementStatus::Failed,
    );

    let mut observed = tracked_snapshot(false, "");
    observed.prompt_id = "prompt-failed".to_string();
    observed.turn_id = "provider-run-1".to_string();
    observed.started_at_ms = Some(300);
    completed.record(CompletedGitTurnSnapshot::new(
        observed.clone(),
        observed,
        None,
        410,
    ));

    let projection = completed
        .latest_projection_for_agent("session-1", "agent-1")
        .expect("failed turn should remain projected");
    assert_eq!(projection.turn_id, "provider-run-1");
    assert_eq!(projection.prompt_id, "prompt-failed");
    assert_eq!(
        projection.settlement_status,
        CompletedTurnSettlementStatus::Failed
    );
    assert!(projection.undo_available);
}

#[test]
fn completed_turns_keep_digests_of_dirty_files_not_their_contents() {
    use super::super::WorkspaceLiveSyncTrackedFileSnapshot;
    let file = |content: &str| WorkspaceLiveSyncTrackedFileSnapshot {
        content_base64: Some(content.repeat(1 << 20)),
        binary: false,
    };
    let completed = CompletedGitTurnSnapshotStore::default();
    let mut before = tracked_snapshot(true, "?? io-1");
    before
        .workspace_live_sync_file_snapshots
        .insert("io-1".into(), file("a"));
    let mut after = before.clone();
    completed.record(CompletedGitTurnSnapshot::new(
        before.clone(),
        after.clone(),
        None,
        200,
    ));
    after
        .workspace_live_sync_file_snapshots
        .insert("io-2".into(), file("b"));
    let mut second = before.clone();
    second.turn_id = "prompt-2".into();
    second.prompt_id = "prompt-2".into();
    completed.record(CompletedGitTurnSnapshot::new(second, after, None, 300));

    let first = completed
        .resolve("session-1", "agent-1", Some("prompt-1"))
        .expect("first turn is recorded");
    let second = completed
        .resolve("session-1", "agent-1", Some("prompt-2"))
        .expect("second turn is recorded");
    for turn in [&first, &second] {
        let retained: usize = [&turn.before, &turn.after]
            .iter()
            .flat_map(|snapshot| snapshot.workspace_live_sync_file_snapshots.values())
            .filter_map(|file| file.content_base64.as_ref())
            .map(String::len)
            .sum();
        assert_eq!(retained, 0, "file contents stay out of the completed turn");
    }
    assert!(first.files_unchanged);
    assert!(!second.files_unchanged);
    assert_ne!(first.artifact_receipt, second.artifact_receipt);
}
