use super::super::{CompletedGitTurnSnapshot, CompletedGitTurnSnapshotStore, GitTurnSnapshotStore};
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
